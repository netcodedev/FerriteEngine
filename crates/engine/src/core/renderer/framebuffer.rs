use super::texture::Texture;

pub struct FrameBuffer {
    id: u32,
    width: u32,
    height: u32,
    depth_texture: Option<Texture>,
}

impl FrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let mut id = 0;
        unsafe {
            gl::GenFramebuffers(1, &mut id);
            gl::BindFramebuffer(gl::FRAMEBUFFER, id);
            gl::DrawBuffer(gl::NONE);
        }
        Self {
            id,
            width,
            height,
            depth_texture: None,
        }
    }

    pub fn append_depth_texture(&mut self, texture: Texture) {
        self.bind();
        unsafe {
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::DEPTH_ATTACHMENT,
                gl::TEXTURE_2D,
                texture.id,
                0,
            );
        }
        self.depth_texture = Some(texture);
        FrameBuffer::unbind();
    }

    pub fn bind(&self) {
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, 0);
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.id);
            gl::Viewport(0, 0, self.width as i32, self.height as i32);
        }
        if let Some(texture) = &self.depth_texture {
            texture.bind();
        }
    }

    pub fn unbind() {
        unsafe {
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
        }
    }

    pub fn depth_only(&self) {
        self.bind();
        unsafe {
            gl::DrawBuffer(gl::NONE);
            gl::ReadBuffer(gl::NONE);
        }
        FrameBuffer::unbind();
    }

    pub fn get_depth_texture(&self) -> Option<&Texture> {
        self.depth_texture.as_ref()
    }
}

impl Drop for FrameBuffer {
    fn drop(&mut self) {
        unsafe {
            gl::DeleteFramebuffers(1, &self.id);
        }
    }
}

pub struct ShadowFrameBuffer(pub FrameBuffer);

impl ShadowFrameBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        let mut fbo = FrameBuffer::new(width, height);
        let texture = Texture::new();
        texture.set_as_depth_texture(width, height);
        fbo.append_depth_texture(texture);
        fbo.depth_only();
        Self(fbo)
    }

    pub fn bind(&self) {
        self.0.bind();
    }

    pub fn get_depth_texture(&self) -> Option<&Texture> {
        self.0.get_depth_texture()
    }
}
