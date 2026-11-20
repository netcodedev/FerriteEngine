//! Instance, device and swapchain.

use std::{
    collections::HashMap,
    ffi::{c_void, CStr, CString},
    sync::Mutex,
};

use ash::vk;

/// What stays the same for the device's life.
pub struct Gpu {
    pub _entry: ash::Entry,
    pub instance: ash::Instance,
    pub debug: Option<(ash::ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    pub surface_fn: ash::khr::surface::Instance,
    pub surface: vk::SurfaceKHR,
    pub physical: vk::PhysicalDevice,
    pub props: vk::PhysicalDeviceProperties,
    pub max_push_descriptors: u32,
    pub device: ash::Device,
    pub queue: vk::Queue,
    pub family: u32,
    pub swapchain_fn: ash::khr::swapchain::Device,
    pub push_fn: ash::khr::push_descriptor::Device,
    /// What `Format::Depth24Stencil8` is made of.
    pub depth_stencil_format: vk::Format,
    pub window_samples: u32,
    pub memory_budget: bool,
    /// `VK_EXT_sample_locations`: multisampling uses GL's sample positions.
    pub sample_locations: bool,
    pub driver: String,
}

fn validation_requested() -> bool {
    match std::env::var("FERRITE_VK_VALIDATION") {
        Ok(v) => v == "1" || v == "true",
        Err(_) => cfg!(debug_assertions),
    }
}

/// Messages seen per id, so a message repeated every frame is logged a
/// few times only.
static SEEN: Mutex<Option<HashMap<i32, u32>>> = Mutex::new(None);

unsafe extern "system" fn on_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    if data.is_null() {
        return vk::FALSE;
    }
    let data = &*data;
    let id = data.message_id_number;
    if let Ok(mut seen) = SEEN.lock() {
        let count = seen.get_or_insert_with(HashMap::new).entry(id).or_insert(0);
        *count += 1;
        if *count > 5 {
            return vk::FALSE;
        }
    }
    let message = if data.p_message.is_null() {
        String::new()
    } else {
        CStr::from_ptr(data.p_message).to_string_lossy().into_owned()
    };
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        log::error!(target: "ferrite::vulkan", "{message}");
    } else if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING) {
        log::warn!(target: "ferrite::vulkan", "{message}");
    } else {
        log::debug!(target: "ferrite::vulkan", "{message}");
    }
    vk::FALSE
}

/// Make the instance (with `extensions`, the window system's), the
/// surface (by `create_surface`) and the device.
pub fn create(
    extensions: &[String],
    create_surface: &dyn Fn(vk::Instance) -> Result<vk::SurfaceKHR, String>,
) -> Result<Gpu, String> {
    unsafe {
        let entry = ash::Entry::load().map_err(|e| format!("no Vulkan loader: {e}"))?;
        let version = entry
            .try_enumerate_instance_version()
            .map_err(|e| format!("instance version: {e}"))?
            .unwrap_or(vk::API_VERSION_1_0);
        if version < vk::API_VERSION_1_3 {
            return Err(format!(
                "Vulkan 1.3 needed, the loader offers {}.{}",
                vk::api_version_major(version),
                vk::api_version_minor(version)
            ));
        }
        let layers_available = entry.enumerate_instance_layer_properties().unwrap_or_default();
        let validation_layer = c"VK_LAYER_KHRONOS_validation";
        let mut validation = validation_requested();
        if validation && !layers_available.iter().any(|l| l.layer_name_as_c_str() == Ok(validation_layer)) {
            log::warn!("Vulkan validation asked for, but the layer is not installed");
            validation = false;
        }
        let mut extension_names: Vec<CString> =
            extensions.iter().filter_map(|e| CString::new(e.as_str()).ok()).collect();
        if validation {
            extension_names.push(CString::from(ash::ext::debug_utils::NAME));
        }
        let extension_ptrs: Vec<*const i8> = extension_names.iter().map(|e| e.as_ptr()).collect();
        let layer_ptrs: Vec<*const i8> = if validation { vec![validation_layer.as_ptr()] } else { Vec::new() };
        let app_name = c"Ferrite";
        let app = vk::ApplicationInfo::default()
            .application_name(app_name)
            .engine_name(app_name)
            .api_version(vk::API_VERSION_1_3);
        let instance = entry
            .create_instance(
                &vk::InstanceCreateInfo::default()
                    .application_info(&app)
                    .enabled_extension_names(&extension_ptrs)
                    .enabled_layer_names(&layer_ptrs),
                None,
            )
            .map_err(|e| format!("vkCreateInstance: {e}"))?;
        let debug = if validation {
            let debug_fn = ash::ext::debug_utils::Instance::new(&entry, &instance);
            let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::ERROR | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                        | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(on_message));
            match debug_fn.create_debug_utils_messenger(&info, None) {
                Ok(m) => {
                    log::info!("Vulkan validation layer on");
                    Some((debug_fn, m))
                }
                Err(e) => {
                    log::warn!("debug messenger: {e}");
                    None
                }
            }
        } else {
            None
        };
        let surface_fn = ash::khr::surface::Instance::new(&entry, &instance);
        let surface = create_surface(instance.handle())?;

        let required: [&CStr; 3] = [
            ash::khr::swapchain::NAME,
            ash::khr::push_descriptor::NAME,
            ash::ext::depth_clip_control::NAME,
        ];
        let mut best: Option<(i32, vk::PhysicalDevice, u32)> = None;
        let mut rejected = Vec::new();
        for physical in instance.enumerate_physical_devices().map_err(|e| format!("devices: {e}"))? {
            let props = instance.get_physical_device_properties(physical);
            let name = props.device_name_as_c_str().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if props.api_version < vk::API_VERSION_1_3 {
                rejected.push(format!("{name}: Vulkan 1.3 not supported"));
                continue;
            }
            let available = instance.enumerate_device_extension_properties(physical).unwrap_or_default();
            let missing: Vec<&CStr> = required
                .iter()
                .copied()
                .filter(|r| !available.iter().any(|a| a.extension_name_as_c_str() == Ok(*r)))
                .collect();
            if !missing.is_empty() {
                rejected.push(format!("{name}: missing {missing:?}"));
                continue;
            }
            let families = instance.get_physical_device_queue_family_properties(physical);
            let family = families.iter().enumerate().position(|(i, f)| {
                f.queue_flags.contains(vk::QueueFlags::GRAPHICS)
                    && surface_fn.get_physical_device_surface_support(physical, i as u32, surface).unwrap_or(false)
            });
            let Some(family) = family else {
                rejected.push(format!("{name}: no queue that draws and presents"));
                continue;
            };
            let mut depth_clip = vk::PhysicalDeviceDepthClipControlFeaturesEXT::default();
            let mut features2 = vk::PhysicalDeviceFeatures2::default().push_next(&mut depth_clip);
            instance.get_physical_device_features2(physical, &mut features2);
            let f = features2.features;
            if depth_clip.depth_clip_control == vk::FALSE
                || f.sampler_anisotropy == vk::FALSE
                || f.image_cube_array == vk::FALSE
                || f.fill_mode_non_solid == vk::FALSE
            {
                rejected.push(format!("{name}: missing features"));
                continue;
            }
            let score = match props.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 100,
                _ => 10,
            };
            if best.is_none_or(|(s, _, _)| score > s) {
                best = Some((score, physical, family as u32));
            }
        }
        let Some((_, physical, family)) = best else {
            return Err(format!("no suitable GPU ({})", rejected.join("; ")));
        };
        let props = instance.get_physical_device_properties(physical);
        let mut push_props = vk::PhysicalDevicePushDescriptorPropertiesKHR::default();
        let mut driver_props = vk::PhysicalDeviceDriverProperties::default();
        let mut props2 = vk::PhysicalDeviceProperties2::default().push_next(&mut push_props).push_next(&mut driver_props);
        instance.get_physical_device_properties2(physical, &mut props2);
        let driver = format!(
            "{} {}",
            driver_props.driver_name_as_c_str().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default(),
            driver_props.driver_info_as_c_str().map(|c| c.to_string_lossy().into_owned()).unwrap_or_default()
        );

        let available = instance.enumerate_device_extension_properties(physical).unwrap_or_default();
        let memory_budget =
            available.iter().any(|a| a.extension_name_as_c_str() == Ok(ash::ext::memory_budget::NAME));
        let mut device_extensions: Vec<*const i8> = required.iter().map(|r| r.as_ptr()).collect();
        if memory_budget {
            device_extensions.push(ash::ext::memory_budget::NAME.as_ptr());
        }
        let sample_locations =
            available.iter().any(|a| a.extension_name_as_c_str() == Ok(ash::ext::sample_locations::NAME));
        if sample_locations {
            device_extensions.push(ash::ext::sample_locations::NAME.as_ptr());
        }
        let features = vk::PhysicalDeviceFeatures::default()
            .sampler_anisotropy(true)
            .image_cube_array(true)
            .fill_mode_non_solid(true);
        let mut features12 = vk::PhysicalDeviceVulkan12Features::default().host_query_reset(true);
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true)
            // `discard` compiles to demoting the invocation.
            .shader_demote_to_helper_invocation(true);
        let mut depth_clip = vk::PhysicalDeviceDepthClipControlFeaturesEXT::default().depth_clip_control(true);
        let priorities = [1.0f32];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(family).queue_priorities(&priorities)];
        let device = instance
            .create_device(
                physical,
                &vk::DeviceCreateInfo::default()
                    .queue_create_infos(&queues)
                    .enabled_extension_names(&device_extensions)
                    .enabled_features(&features)
                    .push_next(&mut features12)
                    .push_next(&mut features13)
                    .push_next(&mut depth_clip),
                None,
            )
            .map_err(|e| format!("vkCreateDevice: {e}"))?;
        let queue = device.get_device_queue(family, 0);
        let swapchain_fn = ash::khr::swapchain::Device::new(&instance, &device);
        let push_fn = ash::khr::push_descriptor::Device::new(&instance, &device);

        let supports = |format: vk::Format, wanted: vk::FormatFeatureFlags| {
            instance.get_physical_device_format_properties(physical, format).optimal_tiling_features.contains(wanted)
        };
        let ds_features = vk::FormatFeatureFlags::DEPTH_STENCIL_ATTACHMENT | vk::FormatFeatureFlags::SAMPLED_IMAGE;
        let depth_stencil_format = if supports(vk::Format::D32_SFLOAT_S8_UINT, ds_features) {
            vk::Format::D32_SFLOAT_S8_UINT
        } else {
            vk::Format::D24_UNORM_S8_UINT
        };
        let counts = props.limits.framebuffer_color_sample_counts & props.limits.framebuffer_depth_sample_counts;
        let window_samples = [8u32, 4, 2]
            .into_iter()
            .find(|&n| counts.contains(super::convert::samples(n)))
            .unwrap_or(1);

        Ok(Gpu {
            _entry: entry,
            instance,
            debug,
            surface_fn,
            surface,
            physical,
            props,
            max_push_descriptors: push_props.max_push_descriptors,
            device,
            queue,
            family,
            swapchain_fn,
            push_fn,
            depth_stencil_format,
            window_samples,
            memory_budget,
            sample_locations,
            driver,
        })
    }
}

impl Gpu {
    pub fn name(&self) -> String {
        self.props.device_name_as_c_str().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Device-local memory `(used, budget)` from `VK_EXT_memory_budget`.
    pub fn memory_budget(&self) -> Option<(u64, u64)> {
        if !self.memory_budget {
            return None;
        }
        unsafe {
            let mut budget = vk::PhysicalDeviceMemoryBudgetPropertiesEXT::default();
            let mut props = vk::PhysicalDeviceMemoryProperties2::default().push_next(&mut budget);
            self.instance.get_physical_device_memory_properties2(self.physical, &mut props);
            let memory = props.memory_properties;
            let (mut used, mut total) = (0, 0);
            for i in 0..memory.memory_heap_count as usize {
                if memory.memory_heaps[i].flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL) {
                    used += budget.heap_usage[i];
                    total += budget.heap_budget[i];
                }
            }
            Some((used, total))
        }
    }
}

pub struct Swapchain {
    pub handle: vk::SwapchainKHR,
    pub images: Vec<vk::Image>,
    pub extent: vk::Extent2D,
    /// Signalled when an image's frame is drawn; one per image, as
    /// presenting holds it until the image comes back.
    pub finished: Vec<vk::Semaphore>,
}

/// A swapchain for the window of `size` pixels, replacing `old`.
pub fn create_swapchain(gpu: &Gpu, old: vk::SwapchainKHR, size: (u32, u32), vsync: bool) -> Result<Swapchain, String> {
    unsafe {
        let caps = gpu
            .surface_fn
            .get_physical_device_surface_capabilities(gpu.physical, gpu.surface)
            .map_err(|e| format!("surface capabilities: {e}"))?;
        let formats = gpu
            .surface_fn
            .get_physical_device_surface_formats(gpu.physical, gpu.surface)
            .map_err(|e| format!("surface formats: {e}"))?;
        let format = formats
            .iter()
            .find(|f| f.format == vk::Format::B8G8R8A8_UNORM && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR)
            .or_else(|| formats.iter().find(|f| f.format == vk::Format::R8G8B8A8_UNORM))
            .or(formats.first())
            .copied()
            .ok_or("the surface offers no formats")?;
        let modes = gpu
            .surface_fn
            .get_physical_device_surface_present_modes(gpu.physical, gpu.surface)
            .unwrap_or_default();
        let mode = if vsync {
            vk::PresentModeKHR::FIFO
        } else if modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
            vk::PresentModeKHR::IMMEDIATE
        } else {
            vk::PresentModeKHR::FIFO
        };
        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: size.0.clamp(caps.min_image_extent.width, caps.max_image_extent.width.max(1)),
                height: size.1.clamp(caps.min_image_extent.height, caps.max_image_extent.height.max(1)),
            }
        };
        let extent = vk::Extent2D { width: extent.width.max(1), height: extent.height.max(1) };
        let mut count = caps.min_image_count + 1;
        if caps.max_image_count > 0 {
            count = count.min(caps.max_image_count);
        }
        let handle = gpu
            .swapchain_fn
            .create_swapchain(
                &vk::SwapchainCreateInfoKHR::default()
                    .surface(gpu.surface)
                    .min_image_count(count)
                    .image_format(format.format)
                    .image_color_space(format.color_space)
                    .image_extent(extent)
                    .image_array_layers(1)
                    .image_usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::COLOR_ATTACHMENT)
                    .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
                    .pre_transform(caps.current_transform)
                    .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE)
                    .present_mode(mode)
                    .clipped(true)
                    .old_swapchain(old),
                None,
            )
            .map_err(|e| format!("vkCreateSwapchainKHR: {e}"))?;
        let images = gpu.swapchain_fn.get_swapchain_images(handle).map_err(|e| format!("swapchain images: {e}"))?;
        let finished = images
            .iter()
            .map(|_| gpu.device.create_semaphore(&vk::SemaphoreCreateInfo::default(), None).unwrap_or_default())
            .collect();
        log::info!(
            "Vulkan swapchain {}×{} {:?}, {} images, {:?}",
            extent.width,
            extent.height,
            format.format,
            images.len(),
            mode
        );
        Ok(Swapchain { handle, images, extent, finished })
    }
}

pub fn destroy_swapchain(gpu: &Gpu, swapchain: &Swapchain) {
    unsafe {
        for &s in &swapchain.finished {
            gpu.device.destroy_semaphore(s, None);
        }
        gpu.swapchain_fn.destroy_swapchain(swapchain.handle, None);
    }
}
