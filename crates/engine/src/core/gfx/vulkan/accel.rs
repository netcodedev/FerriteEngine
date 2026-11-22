//! Acceleration structures, for ray queries (`VK_KHR_acceleration_structure`).
//!
//! A bottom level structure is built once from a mesh's buffers, in the
//! upload commands (which run before the frame's draws). A top level one
//! is built anew whenever asked, in the frame's commands where it is
//! asked, into fresh memory: the frame before may still be reading the
//! last one, which is retired like any other resource.

use ash::vk;
use gpu_allocator::MemoryLocation;

use super::*;

/// An acceleration structure and the memory it lives in.
pub(super) struct Accel {
    pub(super) handle: vk::AccelerationStructureKHR,
    buffer: vk::Buffer,
    allocation: Option<Allocation>,
    size: usize,
    /// Its device address (what a top level instance refers to).
    address: u64,
}

/// Usage of buffers that hold acceleration structures.
const STORAGE_USAGE: vk::BufferUsageFlags = vk::BufferUsageFlags::from_raw(
    vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR.as_raw()
        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS.as_raw(),
);
const SCRATCH_USAGE: vk::BufferUsageFlags = vk::BufferUsageFlags::from_raw(
    vk::BufferUsageFlags::STORAGE_BUFFER.as_raw()
        | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS.as_raw(),
);

impl State {
    pub(super) fn buffer_address(&self, buffer: vk::Buffer) -> u64 {
        unsafe {
            self.gpu
                .device
                .get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
        }
    }

    /// A structure of `ty` big enough for `sizes`, and scratch memory to
    /// build it (retired with this frame).
    fn make_accel(
        &mut self,
        ty: vk::AccelerationStructureTypeKHR,
        sizes: &vk::AccelerationStructureBuildSizesInfoKHR,
    ) -> Option<(Accel, u64)> {
        let accel_fn = self.gpu.accel_fn.clone()?;
        let size = sizes.acceleration_structure_size;
        let (buffer, allocation) = self.raw_buffer(
            size,
            STORAGE_USAGE,
            MemoryLocation::GpuOnly,
            "acceleration structure",
        )?;
        let handle = unsafe {
            accel_fn.create_acceleration_structure(
                &vk::AccelerationStructureCreateInfoKHR::default()
                    .buffer(buffer)
                    .size(size)
                    .ty(ty),
                None,
            )
        };
        let handle = match handle {
            Ok(h) => h,
            Err(e) => {
                self.warn_once(format!("acceleration structure: {e}"));
                self.retire(Retired::Buffer(buffer, allocation, size as usize));
                return None;
            }
        };
        let address = unsafe {
            accel_fn.get_acceleration_structure_device_address(
                &vk::AccelerationStructureDeviceAddressInfoKHR::default()
                    .acceleration_structure(handle),
            )
        };
        let align = self.gpu.scratch_alignment;
        let scratch_size = sizes.build_scratch_size + align;
        let Some((scratch, scratch_allocation)) = self.raw_buffer(
            scratch_size,
            SCRATCH_USAGE,
            MemoryLocation::GpuOnly,
            "acceleration structure scratch",
        ) else {
            unsafe { accel_fn.destroy_acceleration_structure(handle, None) };
            self.retire(Retired::Buffer(buffer, allocation, size as usize));
            return None;
        };
        let scratch_address = self.buffer_address(scratch).next_multiple_of(align);
        // Done with once the build ran.
        self.retire(Retired::Buffer(
            scratch,
            scratch_allocation,
            scratch_size as usize,
        ));
        Some((
            Accel {
                handle,
                buffer,
                allocation: Some(allocation),
                size: size as usize,
                address,
            },
            scratch_address,
        ))
    }

    pub(super) fn create_blas(&mut self, geometries: &[BlasGeometry]) -> RawId {
        let Some(accel_fn) = self.gpu.accel_fn.clone() else {
            return 0;
        };
        let mut built = Vec::with_capacity(geometries.len());
        let mut ranges = Vec::with_capacity(geometries.len());
        let mut counts = Vec::with_capacity(geometries.len());
        for g in geometries {
            let triangles = g.count / 3;
            if triangles == 0 || g.vertex_count == 0 {
                continue;
            }
            let Some((vertices, _)) = self.use_buffer(g.vertices) else {
                continue;
            };
            let indices = match g.indices {
                Some(id) => match self.use_buffer(id) {
                    Some((b, _)) => Some(b),
                    None => continue,
                },
                None => None,
            };
            let mut data = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                .vertex_format(vk::Format::R32G32B32_SFLOAT)
                .vertex_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: self.buffer_address(vertices),
                })
                .vertex_stride(g.stride as u64)
                .max_vertex(g.vertex_count - 1);
            data = match indices {
                Some(b) => data.index_type(vk::IndexType::UINT32).index_data(
                    vk::DeviceOrHostAddressConstKHR {
                        device_address: self.buffer_address(b),
                    },
                ),
                None => data.index_type(vk::IndexType::NONE_KHR),
            };
            built.push(
                vk::AccelerationStructureGeometryKHR::default()
                    .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                    .geometry(vk::AccelerationStructureGeometryDataKHR { triangles: data })
                    .flags(if g.opaque {
                        vk::GeometryFlagsKHR::OPAQUE
                    } else {
                        vk::GeometryFlagsKHR::NO_DUPLICATE_ANY_HIT_INVOCATION
                    }),
            );
            ranges.push(
                vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(triangles),
            );
            counts.push(triangles);
        }
        if built.is_empty() {
            return 0;
        }
        let mut info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&built);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            accel_fn.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &counts,
                &mut sizes,
            );
        }
        let Some((accel, scratch)) =
            self.make_accel(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL, &sizes)
        else {
            return 0;
        };
        info = info.dst_acceleration_structure(accel.handle).scratch_data(
            vk::DeviceOrHostAddressKHR {
                device_address: scratch,
            },
        );
        let upload = self.upload_cmd();
        // After the copies that filled the meshes' buffers.
        self.global_barrier(upload);
        unsafe { accel_fn.cmd_build_acceleration_structures(upload, &[info], &[&ranges]) };
        let id = self.id();
        self.accels.insert(id, accel);
        id
    }

    pub(super) fn build_tlas(&mut self, tlas: RawId, instances: &[TlasInstance]) -> RawId {
        let Some(accel_fn) = self.gpu.accel_fn.clone() else {
            return 0;
        };
        // The instances, in memory the build reads from the host's writes.
        let records: Vec<vk::AccelerationStructureInstanceKHR> = instances
            .iter()
            .filter_map(|i| {
                let blas = self.accels.get(&i.blas)?;
                let m = &i.transform;
                Some(vk::AccelerationStructureInstanceKHR {
                    transform: vk::TransformMatrixKHR {
                        matrix: [
                            m[0][0], m[0][1], m[0][2], m[0][3], m[1][0], m[1][1], m[1][2], m[1][3],
                            m[2][0], m[2][1], m[2][2], m[2][3],
                        ],
                    },
                    instance_custom_index_and_mask: vk::Packed24_8::new(
                        i.custom & 0xFF_FFFF,
                        i.mask,
                    ),
                    instance_shader_binding_table_record_offset_and_flags: vk::Packed24_8::new(
                        0,
                        vk::GeometryInstanceFlagsKHR::TRIANGLE_FACING_CULL_DISABLE.as_raw() as u8,
                    ),
                    acceleration_structure_reference: vk::AccelerationStructureReferenceKHR {
                        device_handle: blas.address,
                    },
                })
            })
            .collect();
        let bytes =
            std::mem::size_of::<vk::AccelerationStructureInstanceKHR>() * records.len().max(1);
        let usage = resources::BUFFER_USAGE
            | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS
            | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        let Some((instance_buffer, instance_allocation)) = self.raw_buffer(
            bytes as u64,
            usage,
            MemoryLocation::CpuToGpu,
            "acceleration structure instances",
        ) else {
            return tlas;
        };
        if let Some(ptr) = instance_allocation.mapped_ptr() {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    records.as_ptr() as *const u8,
                    ptr.as_ptr() as *mut u8,
                    std::mem::size_of_val(records.as_slice()),
                );
            }
        }
        let instance_address = self.buffer_address(instance_buffer);
        self.retire(Retired::Buffer(instance_buffer, instance_allocation, bytes));

        let geometry = [vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR {
                instances: vk::AccelerationStructureGeometryInstancesDataKHR::default().data(
                    vk::DeviceOrHostAddressConstKHR {
                        device_address: instance_address,
                    },
                ),
            })];
        let count = records.len() as u32;
        let mut info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_BUILD)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometry);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        unsafe {
            accel_fn.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &[count],
                &mut sizes,
            );
        }
        let Some((accel, scratch)) =
            self.make_accel(vk::AccelerationStructureTypeKHR::TOP_LEVEL, &sizes)
        else {
            return tlas;
        };
        info = info.dst_acceleration_structure(accel.handle).scratch_data(
            vk::DeviceOrHostAddressKHR {
                device_address: scratch,
            },
        );
        let range = [vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(count)];
        // Outside rendering, after what wrote the bottom level structures,
        // before what traces rays through it.
        self.interrupt_rendering();
        let cmd = self.cmd();
        self.global_barrier(cmd);
        unsafe { accel_fn.cmd_build_acceleration_structures(cmd, &[info], &[&range]) };
        self.global_barrier(cmd);
        let id = if tlas != 0 && self.accels.contains_key(&tlas) {
            self.destroy_accel(tlas);
            tlas
        } else {
            self.id()
        };
        self.accels.insert(id, accel);
        id
    }

    /// Let go of acceleration structure `id` once the GPU is done with it.
    pub(super) fn destroy_accel(&mut self, id: RawId) {
        if let Some(mut accel) = self.accels.remove(&id) {
            self.retire(Retired::Accel(accel.handle));
            if let Some(allocation) = accel.allocation.take() {
                self.retire(Retired::Buffer(accel.buffer, allocation, accel.size));
            }
        }
    }
}
