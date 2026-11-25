//! Buffers, textures, samplers, queries and the per-frame memory.

use std::ops::Range;

use ash::vk;
use gpu_allocator::{
    vulkan::{AllocationCreateDesc, AllocationScheme},
    MemoryLocation,
};

use super::*;

/// Size of a piece of per-frame memory.
pub const CHUNK_SIZE: u64 = 8 << 20;
/// Staging memory a frame may fill before what was recorded is submitted
/// and waited for (loading a scene uploads far more than a frame does).
const STAGING_LIMIT: u64 = 256 << 20;
/// Most bytes of retired host-visible buffers kept for reuse (`spare`):
/// past it they are freed.
pub(super) const SPARE_LIMIT: usize = 64 << 20;

pub(super) const BUFFER_USAGE: vk::BufferUsageFlags = vk::BufferUsageFlags::from_raw(
    vk::BufferUsageFlags::VERTEX_BUFFER.as_raw()
        | vk::BufferUsageFlags::INDEX_BUFFER.as_raw()
        | vk::BufferUsageFlags::UNIFORM_BUFFER.as_raw()
        | vk::BufferUsageFlags::STORAGE_BUFFER.as_raw()
        | vk::BufferUsageFlags::TRANSFER_SRC.as_raw()
        | vk::BufferUsageFlags::TRANSFER_DST.as_raw(),
);

impl State {
    /// Whether a texture is multisampled depth drawn with GL's sample
    /// positions (which its layout changes must name).
    pub(super) fn custom_locations(&self, format: Format, samples: u32) -> bool {
        self.gpu.sample_locations && format.is_depth() && samples > 1
    }

    pub(super) fn raw_buffer(
        &mut self,
        size: u64,
        usage: vk::BufferUsageFlags,
        location: MemoryLocation,
        name: &str,
    ) -> Option<(vk::Buffer, Allocation)> {
        unsafe {
            let buffer = self
                .gpu
                .device
                .create_buffer(&vk::BufferCreateInfo::default().size(size.max(4)).usage(usage), None)
                .ok()?;
            let requirements = self.gpu.device.get_buffer_memory_requirements(buffer);
            let allocation = match self.allocator.allocate(&AllocationCreateDesc {
                name,
                requirements,
                location,
                linear: true,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            }) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("Vulkan: no memory for a buffer of {size} bytes: {e}");
                    self.gpu.device.destroy_buffer(buffer, None);
                    return None;
                }
            };
            if self.gpu.device.bind_buffer_memory(buffer, allocation.memory(), allocation.offset()).is_err() {
                self.gpu.device.destroy_buffer(buffer, None);
                let _ = self.allocator.free(allocation);
                return None;
            }
            Some((buffer, allocation))
        }
    }

    /// Fresh storage for a buffer.
    pub(super) fn storage(&mut self, memory: BufferMemory, size: usize) -> Option<(vk::Buffer, Allocation, *mut u8)> {
        if memory == BufferMemory::Dynamic {
            if let Some((b, a)) = self.spare.get_mut(&size).and_then(|s| s.pop()) {
                self.spare_bytes = self.spare_bytes.saturating_sub(size);
                if self.spare.get(&size).is_some_and(Vec::is_empty) {
                    // Sizes come and go (instance lists while driving).
                    self.spare.remove(&size);
                }
                let ptr = a.mapped_ptr().map_or(std::ptr::null_mut(), |p| p.as_ptr() as *mut u8);
                return Some((b, a, ptr));
            }
        }
        let location = match memory {
            BufferMemory::Static => MemoryLocation::GpuOnly,
            BufferMemory::Dynamic => MemoryLocation::CpuToGpu,
        };
        // With ray tracing, meshes may be built into acceleration
        // structures: by their device address.
        let usage = if self.gpu.accel_fn.is_some() {
            BUFFER_USAGE
                | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
        } else {
            BUFFER_USAGE
        };
        let name = match memory {
            BufferMemory::Static => "buffer (GPU)",
            BufferMemory::Dynamic => "buffer (host-visible, written by the CPU)",
        };
        let (b, a) = self.raw_buffer(size as u64, usage, location, name)?;
        let ptr = a.mapped_ptr().map_or(std::ptr::null_mut(), |p| p.as_ptr() as *mut u8);
        Some((b, a, ptr))
    }

    /// `size` bytes of this frame's host-visible memory, aligned to
    /// `align`: buffer, offset, pointer.
    pub(super) fn arena_alloc(&mut self, size: u64, align: u64) -> Option<(vk::Buffer, u64, *mut u8)> {
        let frame = self.frame;
        for chunk in &mut self.frames[frame].arena {
            let offset = chunk.used.next_multiple_of(align);
            if offset + size <= chunk.size {
                chunk.used = offset + size;
                return Some((chunk.buffer, offset, unsafe { chunk.ptr.add(offset as usize) }));
            }
        }
        let chunk_size = if size <= CHUNK_SIZE { CHUNK_SIZE } else { size.next_power_of_two() };
        let (buffer, allocation) = self.raw_buffer(
            chunk_size,
            vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::UNIFORM_BUFFER,
            // Only read by transfer copies: plain cached system memory.
            // `CpuToGpu` picks the GPU's own memory behind the PCIe window
            // where it can (resizable BAR); writing hundreds of MB of
            // freshly mapped pages there crawled at ~100 MB/s and stalled
            // the editor for seconds while scenes and thumbnails loaded.
            MemoryLocation::GpuToCpu,
            "frame memory",
        )?;
        let ptr = allocation.mapped_ptr()?.as_ptr() as *mut u8;
        self.frames[frame].arena.push(Chunk { buffer, allocation, ptr, size: chunk_size, used: size });
        Some((buffer, 0, ptr))
    }

    /// `size` bytes of this frame's params memory (one buffer per frame,
    /// so a program's params set can stay the same all frame): buffer,
    /// offset and where to write. Outgrown, it is replaced by one twice
    /// as big (what was drawn with the old one keeps it until done).
    pub(super) fn params_alloc(&mut self, size: u64, align: u64) -> Option<(vk::Buffer, u64, *mut u8)> {
        let frame = self.frame;
        if let Some(chunk) = self.frames[frame].params.as_mut() {
            let offset = chunk.used.next_multiple_of(align);
            if offset + size <= chunk.size {
                chunk.used = offset + size;
                return Some((chunk.buffer, offset, unsafe { chunk.ptr.add(offset as usize) }));
            }
        }
        let old = self.frames[frame].params.take();
        // What went into the old memory is uploaded again into the new.
        for p in self.programs.values_mut() {
            for b in &mut p.blocks {
                b.upload = None;
            }
        }
        let chunk_size = old.as_ref().map_or(16 << 20, |c| c.size * 2).max(size.next_power_of_two());
        if let Some(old) = old {
            let sets: Vec<vk::DescriptorSet> = self
                .params_sets
                .iter()
                .filter(|((_, b), _)| *b == old.buffer)
                .map(|(_, s)| *s)
                .collect();
            self.params_sets.retain(|(_, b), _| *b != old.buffer);
            self.retire(Retired::Sets(sets));
            self.retire(Retired::Buffer(old.buffer, old.allocation, old.size as usize));
        }
        let (buffer, allocation) =
            self.raw_buffer(chunk_size, vk::BufferUsageFlags::UNIFORM_BUFFER, MemoryLocation::CpuToGpu, "params memory")?;
        let ptr = allocation.mapped_ptr()?.as_ptr() as *mut u8;
        self.frames[frame].params = Some(Chunk { buffer, allocation, ptr, size: chunk_size, used: size });
        Some((buffer, 0, ptr))
    }

    /// `data` in this frame's staging memory: buffer and offset.
    fn stage(&mut self, data: &[u8]) -> Option<(vk::Buffer, u64)> {
        let (buffer, offset, ptr) = self.arena_alloc(data.len() as u64, 16)?;
        crate::core::spike_probe::STAGED_BYTES.fetch_add(data.len() as u64, std::sync::atomic::Ordering::Relaxed);
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len()) };
        Some((buffer, offset))
    }

    /// Before a large upload: when this frame's staging memory is full,
    /// submit and wait so it can be used again.
    fn make_room(&mut self, size: u64) {
        if self.arena_used() + size > STAGING_LIMIT && self.arena_used() > 0 {
            self.flush_and_wait();
        }
    }

    /// Copy in the upload commands (which run before the frame's draws).
    fn upload_copy(&mut self, src: vk::Buffer, src_offset: u64, dst: vk::Buffer, dst_offset: u64, size: u64) {
        let cmd = self.upload_cmd();
        if self.upload_dirty.contains(&src) || self.upload_dirty.contains(&dst) {
            self.global_barrier(cmd);
            self.upload_dirty.clear();
        }
        unsafe {
            self.gpu.device.cmd_copy_buffer(
                cmd,
                src,
                dst,
                &[vk::BufferCopy { src_offset, dst_offset, size }],
            );
        }
        self.upload_dirty.insert(dst);
    }

    pub(super) fn create_buffer(
        &mut self,
        _usage: BufferUsage,
        memory: BufferMemory,
        size: usize,
        data: Option<&[u8]>,
    ) -> RawId {
        let Some((buffer, allocation, ptr)) = self.storage(memory, size) else {
            return 0;
        };
        let id = self.id();
        self.buffers.insert(
            id,
            Buf { buffer, allocation: Some(allocation), size, memory, ptr, high_water: 0, used: 0 },
        );
        if let Some(data) = data {
            self.write_buffer(id, 0, data, false);
        }
        id
    }

    pub(super) fn write_buffer(&mut self, id: RawId, offset: usize, data: &[u8], orphan: bool) {
        if data.is_empty() {
            return;
        }
        let Some(buf) = self.buffers.get(&id) else {
            return;
        };
        if offset + data.len() > buf.size {
            self.warn_once(format!("buffer write past its end ({} + {} > {})", offset, data.len(), buf.size));
            return;
        }
        if buf.memory == BufferMemory::Static {
            self.make_room(data.len() as u64);
        }
        let buf = &self.buffers[&id];
        let in_use = match buf.memory {
            // Host writes happen now: any submission not finished may read it.
            BufferMemory::Dynamic => buf.used > self.completed,
            // Uploads run before this frame's draws (and after the last
            // frame's): only this frame's draws may see the new data early.
            BufferMemory::Static => buf.used >= self.serial,
        };
        if orphan || (in_use && offset < buf.high_water) {
            let (memory, size) = (buf.memory, buf.size);
            let Some((nb, na, nptr)) = self.storage(memory, size) else {
                return;
            };
            let buf = self.buffers.get_mut(&id).expect("looked up above");
            let old = (buf.buffer, buf.allocation.take(), buf.ptr);
            buf.buffer = nb;
            buf.allocation = Some(na);
            buf.ptr = nptr;
            buf.high_water = 0;
            buf.used = 0;
            if !orphan {
                match memory {
                    BufferMemory::Dynamic => unsafe { std::ptr::copy_nonoverlapping(old.2, nptr, size) },
                    BufferMemory::Static => self.upload_copy(old.0, 0, nb, 0, size as u64),
                }
            }
            if let Some(a) = old.1 {
                self.retire(Retired::Buffer(old.0, a, size));
            }
        }
        let buf = &self.buffers[&id];
        match buf.memory {
            BufferMemory::Dynamic => unsafe {
                std::ptr::copy_nonoverlapping(data.as_ptr(), buf.ptr.add(offset), data.len());
            },
            BufferMemory::Static => {
                let dst = buf.buffer;
                if let Some((src, src_offset)) = self.stage(data) {
                    self.upload_copy(src, src_offset, dst, offset as u64, data.len() as u64);
                }
            }
        }
        let buf = self.buffers.get_mut(&id).expect("looked up above");
        buf.high_water = buf.high_water.max(offset + data.len());
    }

    pub(super) fn resize_buffer(&mut self, id: RawId, size: usize) {
        let Some(memory) = self.buffers.get(&id).map(|b| b.memory) else {
            return;
        };
        let Some((nb, na, nptr)) = self.storage(memory, size) else {
            return;
        };
        let buf = self.buffers.get_mut(&id).expect("looked up above");
        let old = (buf.buffer, buf.allocation.take(), buf.size);
        *buf = Buf { buffer: nb, allocation: Some(na), size, memory, ptr: nptr, high_water: 0, used: 0 };
        if let Some(a) = old.1 {
            self.retire(Retired::Buffer(old.0, a, old.2));
        }
    }

    /// The buffer's storage now, marked as used by the commands recorded.
    pub(super) fn use_buffer(&mut self, id: RawId) -> Option<(vk::Buffer, usize)> {
        let serial = self.serial;
        let buf = self.buffers.get_mut(&id)?;
        buf.used = serial;
        Some((buf.buffer, buf.size))
    }

    pub(super) fn create_texture(&mut self, desc: &TextureDesc) -> RawId {
        let format = convert::format(desc.format, self.gpu.depth_stencil_format);
        let samples = desc.samples.max(1);
        let mips = if samples > 1 { 1 } else { desc.mip_count() };
        let layers = convert::array_layers(desc);
        let features = unsafe {
            self.gpu.instance.get_physical_device_format_properties(self.gpu.physical, format).optimal_tiling_features
        };
        let mut usage = vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST;
        if samples == 1 {
            usage |= vk::ImageUsageFlags::SAMPLED;
        }
        if desc.kind != TextureKind::D3 {
            if desc.format.is_depth() {
                usage |= vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT;
            } else if features.contains(vk::FormatFeatureFlags::COLOR_ATTACHMENT) {
                usage |= vk::ImageUsageFlags::COLOR_ATTACHMENT;
            }
        }
        let (image_type, depth, array) = match desc.kind {
            TextureKind::D3 => (vk::ImageType::TYPE_3D, desc.layers.max(1), 1),
            _ => (vk::ImageType::TYPE_2D, 1, layers),
        };
        let mut flags = if desc.kind == TextureKind::CubeArray {
            vk::ImageCreateFlags::CUBE_COMPATIBLE
        } else {
            vk::ImageCreateFlags::empty()
        };
        if self.custom_locations(desc.format, samples) {
            flags |= vk::ImageCreateFlags::SAMPLE_LOCATIONS_COMPATIBLE_DEPTH_EXT;
        }
        let info = vk::ImageCreateInfo::default()
            .flags(flags)
            .image_type(image_type)
            .format(format)
            .extent(vk::Extent3D { width: desc.width.max(1), height: desc.height.max(1), depth })
            .mip_levels(mips)
            .array_layers(array)
            .samples(convert::samples(samples))
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        unsafe {
            let image = match self.gpu.device.create_image(&info, None) {
                Ok(i) => i,
                Err(e) => {
                    log::error!("Vulkan: texture {desc:?}: {e}");
                    return 0;
                }
            };
            let requirements = self.gpu.device.get_image_memory_requirements(image);
            let name = if usage.contains(vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT) {
                "texture: depth"
            } else if desc.kind == TextureKind::D3 {
                "texture: 3D"
            } else if layers > 1 {
                "texture: array or cube"
            } else {
                "texture: 2D"
            };
            let allocation = match self.allocator.allocate(&AllocationCreateDesc {
                name,
                requirements,
                location: MemoryLocation::GpuOnly,
                linear: false,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            }) {
                Ok(a) => a,
                Err(e) => {
                    log::error!("Vulkan: no memory for texture {desc:?}: {e}");
                    self.gpu.device.destroy_image(image, None);
                    return 0;
                }
            };
            let _ = self.gpu.device.bind_image_memory(image, allocation.memory(), allocation.offset());
            let view = if samples == 1 {
                self.gpu
                    .device
                    .create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(convert::view_type(desc.kind))
                            .format(format)
                            .subresource_range(vk::ImageSubresourceRange {
                                aspect_mask: convert::sampled_aspect(desc.format),
                                base_mip_level: 0,
                                level_count: mips,
                                base_array_layer: 0,
                                layer_count: array,
                            }),
                        None,
                    )
                    .unwrap_or_default()
            } else {
                vk::ImageView::null()
            };
            let id = self.id();
            let mut desc = *desc;
            desc.samples = samples;
            self.textures.insert(
                id,
                Tex {
                    image,
                    allocation: Some(allocation),
                    desc,
                    format,
                    mips,
                    layers: array,
                    layouts: vec![vk::ImageLayout::UNDEFINED; (mips * array) as usize],
                    view,
                    attachment_views: FxHashMap::default(),
                },
            );
            id
        }
    }

    /// The view of `mip` and `layer` (all layers: `None`) for drawing into.
    pub(super) fn attachment_view(&mut self, id: RawId, mip: u32, layer: Option<u32>) -> vk::ImageView {
        let Some(tex) = self.textures.get(&id) else {
            return vk::ImageView::null();
        };
        if let Some(&v) = tex.attachment_views.get(&(mip, layer)) {
            return v;
        }
        let (view_type, base, count) = match (layer, tex.desc.kind) {
            (Some(l), _) => (vk::ImageViewType::TYPE_2D, l, 1),
            (None, TextureKind::D2) | (None, TextureKind::D3) => (vk::ImageViewType::TYPE_2D, 0, 1),
            (None, _) => (vk::ImageViewType::TYPE_2D_ARRAY, 0, tex.layers),
        };
        let view = unsafe {
            self.gpu
                .device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(tex.image)
                        .view_type(view_type)
                        .format(tex.format)
                        .subresource_range(vk::ImageSubresourceRange {
                            aspect_mask: convert::aspects(tex.desc.format),
                            base_mip_level: mip,
                            level_count: 1,
                            base_array_layer: base,
                            layer_count: count,
                        }),
                    None,
                )
                .unwrap_or_default()
        };
        if let Some(tex) = self.textures.get_mut(&id) {
            tex.attachment_views.insert((mip, layer), view);
        }
        view
    }

    /// Record layout changes (with a global memory barrier) in `cmd`:
    /// texture, mips, layers, new layout.
    pub(super) fn transition(&mut self, cmd: vk::CommandBuffer, list: &[(RawId, Range<u32>, Range<u32>, vk::ImageLayout)]) {
        let mut barriers = Vec::new();
        // Which barriers are of multisampled depth (with GL's positions).
        let mut located = Vec::new();
        for (id, mips, layers, layout) in list {
            let Some(tex) = self.textures.get_mut(id) else {
                continue;
            };
            let aspect = convert::aspects(tex.desc.format);
            let mips = mips.start.min(tex.mips)..mips.end.min(tex.mips);
            let layers = layers.start.min(tex.layers)..layers.end.min(tex.layers);
            let index = |m: u32, l: u32| (m * tex.layers + l) as usize;
            let mut olds = Vec::new();
            for m in mips.clone() {
                for l in layers.clone() {
                    olds.push(tex.layouts[index(m, l)]);
                }
            }
            if olds.iter().all(|o| o == layout) {
                continue;
            }
            let barrier = |old: vk::ImageLayout, m: Range<u32>, l: Range<u32>| {
                vk::ImageMemoryBarrier2::default()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                    .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
                    .old_layout(old)
                    .new_layout(*layout)
                    .image(tex.image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: aspect,
                        base_mip_level: m.start,
                        level_count: m.end - m.start,
                        base_array_layer: l.start,
                        layer_count: l.end - l.start,
                    })
            };
            let custom = self.gpu.sample_locations && tex.desc.format.is_depth() && tex.desc.samples > 1;
            let before = barriers.len();
            if olds.iter().all(|o| *o == olds[0]) {
                barriers.push(barrier(olds[0], mips.clone(), layers.clone()));
            } else {
                for m in mips.clone() {
                    for l in layers.clone() {
                        let old = tex.layouts[index(m, l)];
                        if old != *layout {
                            barriers.push(barrier(old, m..m + 1, l..l + 1));
                        }
                    }
                }
            }
            for m in mips.clone() {
                for l in layers.clone() {
                    tex.layouts[index(m, l)] = *layout;
                }
            }
            for _ in before..barriers.len() {
                located.push(custom.then_some(tex.desc.samples));
            }
        }
        let locations: Vec<Vec<vk::SampleLocationEXT>> =
            located.iter().map(|s| s.map(convert::gl_sample_locations).unwrap_or_default()).collect();
        let mut infos: Vec<vk::SampleLocationsInfoEXT> = located
            .iter()
            .zip(&locations)
            .map(|(s, l)| {
                vk::SampleLocationsInfoEXT::default()
                    .sample_locations_per_pixel(convert::samples(s.unwrap_or(1)))
                    .sample_location_grid_size(vk::Extent2D { width: 1, height: 1 })
                    .sample_locations(l)
            })
            .collect();
        let barriers: Vec<_> = barriers
            .into_iter()
            .zip(infos.iter_mut())
            .zip(&located)
            .map(|((b, info), s)| if s.is_some() { b.push_next(info) } else { b })
            .collect();
        let memory = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)];
        unsafe {
            self.gpu.device.cmd_pipeline_barrier2(
                cmd,
                &vk::DependencyInfo::default().memory_barriers(&memory).image_memory_barriers(&barriers),
            );
        }
    }

    /// The whole of a texture, in `layout`.
    pub(super) fn transition_all(&mut self, cmd: vk::CommandBuffer, ids: &[RawId], layout: vk::ImageLayout) {
        let list: Vec<_> = ids.iter().map(|&id| (id, 0..u32::MAX, 0..u32::MAX, layout)).collect();
        self.transition(cmd, &list);
    }

    pub(super) fn write_texture(
        &mut self,
        id: RawId,
        mip: u32,
        region: Rect,
        layer: u32,
        layers: u32,
        upload: UploadFormat,
        data: &[u8],
    ) {
        let Some(tex) = self.textures.get(&id) else {
            return;
        };
        let desc = tex.desc;
        let data = &data[..upload.data_size(region.width, region.height, layers).min(data.len())];
        let converted = convert::convert_texels(data, upload, desc.format);
        let bytes = converted.as_deref().unwrap_or(data);
        self.make_room(bytes.len() as u64);
        self.interrupt_rendering();
        let Some((buffer, offset)) = self.stage(bytes) else {
            return;
        };
        let cmd = self.cmd();
        let (layer_range, offset_z, depth, base_layer, layer_count) = match desc.kind {
            TextureKind::D3 => (0..1, layer as i32, layers.max(1), 0, 1),
            _ => (layer..layer + layers.max(1), 0, 1, layer, layers.max(1)),
        };
        self.transition(cmd, &[(id, mip..mip + 1, layer_range, vk::ImageLayout::TRANSFER_DST_OPTIMAL)]);
        let tex = &self.textures[&id];
        let region = vk::BufferImageCopy {
            buffer_offset: offset,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: convert::sampled_aspect(desc.format),
                mip_level: mip,
                base_array_layer: base_layer,
                layer_count,
            },
            image_offset: vk::Offset3D { x: region.x, y: region.y, z: offset_z },
            image_extent: vk::Extent3D { width: region.width, height: region.height, depth },
        };
        unsafe {
            self.gpu.device.cmd_copy_buffer_to_image(
                cmd,
                buffer,
                tex.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
    }

    pub(super) fn generate_mips(&mut self, id: RawId) {
        let Some(tex) = self.textures.get(&id) else {
            return;
        };
        let (desc, mips, layers, image) = (tex.desc, tex.mips, tex.layers, tex.image);
        if mips <= 1 {
            return;
        }
        self.interrupt_rendering();
        let cmd = self.cmd();
        let aspect = convert::sampled_aspect(desc.format);
        let filter = if desc.format.is_depth() { vk::Filter::NEAREST } else { vk::Filter::LINEAR };
        let size = |m: u32| {
            let d = if desc.kind == TextureKind::D3 { (desc.layers >> m).max(1) } else { 1 };
            [(desc.width >> m).max(1) as i32, (desc.height >> m).max(1) as i32, d as i32]
        };
        for m in 1..mips {
            self.transition(
                cmd,
                &[
                    (id, m - 1..m, 0..layers, vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                    (id, m..m + 1, 0..layers, vk::ImageLayout::TRANSFER_DST_OPTIMAL),
                ],
            );
            let (s, d) = (size(m - 1), size(m));
            let blit = vk::ImageBlit {
                src_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: aspect,
                    mip_level: m - 1,
                    base_array_layer: 0,
                    layer_count: layers,
                },
                src_offsets: [vk::Offset3D::default(), vk::Offset3D { x: s[0], y: s[1], z: s[2] }],
                dst_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: aspect,
                    mip_level: m,
                    base_array_layer: 0,
                    layer_count: layers,
                },
                dst_offsets: [vk::Offset3D::default(), vk::Offset3D { x: d[0], y: d[1], z: d[2] }],
            };
            unsafe {
                self.gpu.device.cmd_blit_image(
                    cmd,
                    image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit],
                    filter,
                );
            }
        }
    }

    /// The Vulkan sampler for `desc`, made once.
    pub(super) fn sampler_for(&mut self, desc: &SamplerDesc) -> vk::Sampler {
        if let Some(&(_, s)) = self.sampler_variants.iter().find(|(d, _)| d == desc) {
            return s;
        }
        let max_anisotropy = self.gpu.props.limits.max_sampler_anisotropy;
        let border = match desc.border {
            BorderColor::TransparentBlack => vk::BorderColor::FLOAT_TRANSPARENT_BLACK,
            BorderColor::OpaqueBlack => vk::BorderColor::FLOAT_OPAQUE_BLACK,
            BorderColor::OpaqueWhite => vk::BorderColor::FLOAT_OPAQUE_WHITE,
        };
        let info = vk::SamplerCreateInfo::default()
            .mag_filter(convert::filter(desc.mag))
            .min_filter(convert::filter(desc.min))
            // Without mips: NEAREST, so the LOD clamped to 0.25 below rounds
            // to level 0 (with LINEAR it would blend in a quarter of level 1).
            .mipmap_mode(match desc.mip {
                Some(Filter::Linear) => vk::SamplerMipmapMode::LINEAR,
                _ => vk::SamplerMipmapMode::NEAREST,
            })
            .address_mode_u(convert::wrap(desc.wrap[0]))
            .address_mode_v(convert::wrap(desc.wrap[1]))
            .address_mode_w(convert::wrap(desc.wrap[2]))
            .anisotropy_enable(desc.anisotropy > 1)
            .max_anisotropy((desc.anisotropy.max(1) as f32).min(max_anisotropy))
            .compare_enable(desc.compare.is_some())
            .compare_op(convert::compare(desc.compare.unwrap_or(CompareFunc::LessEqual)))
            .min_lod(0.0)
            // GL's filtering without mips reads level 0 only.
            .max_lod(if desc.mip.is_some() { vk::LOD_CLAMP_NONE } else { 0.25 })
            .border_color(border);
        let sampler = unsafe { self.gpu.device.create_sampler(&info, None) }.unwrap_or_default();
        self.sampler_variants.push((*desc, sampler));
        sampler
    }

    /// Textures, buffers for what draws read but nobody bound.
    pub(super) fn create_dummies(&mut self) {
        let kinds = [
            (TextureKind::D2, false),
            (TextureKind::D2Array, false),
            (TextureKind::D3, false),
            (TextureKind::CubeArray, false),
            (TextureKind::D2, true),
            (TextureKind::D2Array, true),
            (TextureKind::CubeArray, true),
        ];
        let cmd = self.cmd();
        for (kind, depth) in kinds {
            let desc = TextureDesc {
                kind,
                format: if depth { Format::Depth32F } else { Format::Rgba8 },
                width: 1,
                height: 1,
                layers: if kind == TextureKind::CubeArray { 6 } else { 1 },
                mips: 1,
                samples: 1,
            };
            let id = self.create_texture(&desc);
            self.transition_all(cmd, &[id], vk::ImageLayout::TRANSFER_DST_OPTIMAL);
            let tex = &self.textures[&id];
            let range = vk::ImageSubresourceRange {
                aspect_mask: convert::aspects(desc.format),
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: tex.layers,
            };
            unsafe {
                if depth {
                    self.gpu.device.cmd_clear_depth_stencil_image(
                        cmd,
                        tex.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &vk::ClearDepthStencilValue { depth: 1.0, stencil: 0 },
                        &[range],
                    );
                } else {
                    self.gpu.device.cmd_clear_color_image(
                        cmd,
                        tex.image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &vk::ClearColorValue { float32: [0.0, 0.0, 0.0, 1.0] },
                        &[range],
                    );
                }
            }
            self.dummy_textures.insert((kind, depth), id);
        }
        self.dummy_buffer = self.create_buffer(BufferUsage::UNIFORM, BufferMemory::Static, 65536, Some(&[0u8; 65536]));
        let attribute: Vec<u8> = [0.0f32, 0.0, 0.0, 1.0].iter().flat_map(|f| f.to_le_bytes()).collect();
        self.default_attribute = self.create_buffer(BufferUsage::VERTEX, BufferMemory::Static, 16, Some(&attribute));
        if self.gpu.accel_fn.is_some() {
            self.dummy_tlas = self.build_tlas(0, &[], false);
        }
    }

    /// Make the window's target the window's size (when it changed).
    pub(super) fn update_window_target(&mut self) {
        let size = super::super::window_size();
        if self.window.as_ref().is_some_and(|w| w.size == size) {
            return;
        }
        self.interrupt_rendering();
        if let Some(old) = self.window.take() {
            for id in [old.color, old.depth, old.resolve] {
                self.destroy(Raw::Texture(id));
            }
        }
        let samples = self.gpu.window_samples;
        let color = self.create_texture(&TextureDesc::d2(Format::Rgba8, size.0, size.1).with_samples(samples));
        let depth = self.create_texture(&TextureDesc::d2(Format::Depth24Stencil8, size.0, size.1).with_samples(samples));
        let resolve = self.create_texture(&TextureDesc::d2(Format::Rgba8, size.0, size.1));
        self.window = Some(WindowTarget { color, depth, resolve, size });
    }

    pub(super) fn create_queries(&mut self, count: u32) -> RawId {
        let pool = unsafe {
            self.gpu.device.create_query_pool(
                &vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count(count.max(1)),
                None,
            )
        };
        let Ok(pool) = pool else {
            return 0;
        };
        unsafe { self.gpu.device.reset_query_pool(pool, 0, count.max(1)) };
        let id = self.id();
        self.queries.insert(id, QuerySet { pool, written: vec![0; count.max(1) as usize] });
        id
    }

    pub(super) fn write_timestamp(&mut self, id: RawId, index: u32) {
        let Some(q) = self.queries.get(&id) else {
            return;
        };
        let Some(&written) = q.written.get(index as usize) else {
            return;
        };
        let pool = q.pool;
        if written != 0 {
            // It must be reset before it is written again: in the upload
            // commands (before this frame's), or here if this frame wrote
            // it already.
            if written >= self.serial {
                self.interrupt_rendering();
                unsafe { self.gpu.device.cmd_reset_query_pool(self.cmd(), pool, index, 1) };
            } else {
                unsafe { self.gpu.device.cmd_reset_query_pool(self.upload_cmd(), pool, index, 1) };
            }
        }
        unsafe {
            self.gpu.device.cmd_write_timestamp2(self.cmd(), vk::PipelineStageFlags2::ALL_COMMANDS, pool, index);
        }
        let serial = self.serial;
        if let Some(q) = self.queries.get_mut(&id) {
            q.written[index as usize] = serial;
        }
    }

    pub(super) fn read_timestamps(&mut self, id: RawId, count: u32) -> Option<Vec<u64>> {
        self.poll_completed();
        let q = self.queries.get(&id)?;
        let count = (count as usize).min(q.written.len());
        if count == 0 || q.written[..count].iter().any(|&w| w == 0 || w > self.completed) {
            return None;
        }
        let mut data = vec![[0u64; 2]; count];
        unsafe {
            self.gpu
                .device
                .get_query_pool_results(
                    q.pool,
                    0,
                    &mut data,
                    vk::QueryResultFlags::TYPE_64 | vk::QueryResultFlags::WITH_AVAILABILITY,
                )
                .ok()?;
        }
        if data.iter().any(|d| d[1] == 0) {
            return None;
        }
        let period = self.gpu.props.limits.timestamp_period as f64;
        Some(data.iter().map(|d| (d[0] as f64 * period) as u64).collect())
    }

    pub(super) fn destroy(&mut self, raw: Raw) {
        match raw {
            Raw::Buffer(id) => {
                if let Some(mut b) = self.buffers.remove(&id) {
                    if let Some(a) = b.allocation.take() {
                        self.retire(Retired::Buffer(b.buffer, a, b.size));
                    }
                }
            }
            Raw::Texture(id) => {
                if let Some(mut t) = self.textures.remove(&id) {
                    let mut views: Vec<vk::ImageView> = t.attachment_views.values().copied().collect();
                    if t.view != vk::ImageView::null() {
                        views.push(t.view);
                    }
                    if let Some(a) = t.allocation.take() {
                        self.retire(Retired::Image(t.image, a, views));
                    }
                }
            }
            Raw::Sampler(_) => {}
            Raw::Target(id) => {
                self.targets.remove(&id);
            }
            Raw::Program(id) => {
                if let Some(p) = self.programs.remove(&id) {
                    self.retire(Retired::Program(p.vertex, p.fragment, p.set_layout, p.layout));
                    if p.params_layout != vk::DescriptorSetLayout::null() {
                        self.retire(Retired::SetLayout(p.params_layout));
                    }
                    let sets: Vec<vk::DescriptorSet> =
                        self.params_sets.iter().filter(|((prog, _), _)| *prog == id).map(|(_, s)| *s).collect();
                    self.params_sets.retain(|(prog, _), _| *prog != id);
                    if !sets.is_empty() {
                        self.retire(Retired::Sets(sets));
                    }
                }
                if self.last_vk_pipeline.is_some_and(|(k, _)| k.program == id) {
                    self.last_vk_pipeline = None;
                }
                let keys: Vec<PipelineKey> = self.vk_pipelines.keys().filter(|k| k.program == id).cloned().collect();
                for k in keys {
                    if let Some(p) = self.vk_pipelines.remove(&k) {
                        self.retire(Retired::Pipeline(p));
                    }
                }
            }
            Raw::Pipeline(id) => {
                self.pipelines.remove(&id);
            }
            Raw::Queries(id) => {
                if let Some(q) = self.queries.remove(&id) {
                    self.retire(Retired::Queries(q.pool));
                }
            }
            Raw::Accel(id) => self.destroy_accel(id),
        }
    }
}
