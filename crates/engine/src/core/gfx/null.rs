//! The device where nothing is drawn: the headless server, tests, and
//! threads other than the render thread.

use super::*;

pub(super) struct NullDevice;

pub(super) static NULL: NullDevice = NullDevice;

impl Device for NullDevice {
    fn kind(&self) -> BackendKind {
        BackendKind::Null
    }
    fn describe(&self) -> String {
        "no GPU".into()
    }
    fn create_buffer(&self, _: BufferUsage, _: BufferMemory, _: usize, _: Option<&[u8]>) -> RawId {
        0
    }
    fn write_buffer(&self, _: RawId, _: usize, _: &[u8], _: bool) {}
    fn resize_buffer(&self, _: RawId, _: usize) {}
    fn create_texture(&self, _: &TextureDesc) -> RawId {
        0
    }
    fn write_texture(&self, _: RawId, _: &TextureDesc, _: u32, _: Rect, _: u32, _: u32, _: UploadFormat, _: &[u8]) {}
    fn generate_mips(&self, _: RawId, _: &TextureDesc) {}
    fn create_sampler(&self, _: &SamplerDesc) -> RawId {
        0
    }
    fn create_target(&self, _: &[Attachment], _: Option<Attachment>) -> RawId {
        0
    }
    fn create_program(&self, _: &str, _: &str, _: &str) -> Result<RawId, String> {
        Ok(0)
    }
    fn create_pipeline(&self, _: RawId, _: &PipelineDesc) -> RawId {
        0
    }
    fn create_queries(&self, _: u32) -> RawId {
        0
    }
    fn write_timestamp(&self, _: RawId, _: u32) {}
    fn read_timestamps(&self, _: RawId, _: u32) -> Option<Vec<u64>> {
        None
    }
    fn destroy(&self, _: Raw) {}
    fn begin_pass(&self, _: TargetRef, _: (u32, u32), _: Load, _: Load) {}
    fn resume_pass(&self, _: TargetRef, _: (u32, u32)) {}
    fn end_pass(&self) {}
    fn set_viewport(&self, _: Rect) {}
    fn set_scissor(&self, _: Option<Rect>) {}
    fn set_pipeline(&self, _: RawId, _: RawId) {}
    fn set_stencil_reference(&self, _: u8) {}
    fn bind_texture(&self, _: u32, _: RawId, _: TextureKind, _: RawId) {}
    fn unbind_texture(&self, _: u32, _: TextureKind) {}
    fn bind_uniform_buffer(&self, _: u32, _: RawId, _: usize, _: usize) {}
    fn bind_storage_buffer(&self, _: u32, _: RawId, _: usize, _: usize) {}
    fn set_vertex_buffer(&self, _: u32, _: RawId, _: usize, _: u32) {}
    fn set_index_buffer(&self, _: RawId, _: IndexFormat) {}
    fn draw(&self, _: u32, _: u32, _: u32, _: u32) {}
    fn draw_indexed(&self, _: u32, _: u32, _: i32, _: u32, _: u32) {}
    fn clear(&self, _: Planes, _: [f32; 4], _: f32, _: u8) {}
    fn blit(&self, _: TargetRef, _: Rect, _: TargetRef, _: Rect, _: Planes, _: bool) {}
    fn read_pixels(&self, _: TargetRef, rect: Rect) -> Vec<u8> {
        vec![0; rect.width as usize * rect.height as usize * 4]
    }
    fn begin_frame(&self) {}
    fn end_frame(&self) {}
}
