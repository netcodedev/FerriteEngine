//! Passes, draws, clears, blits, readback and presenting.

use ash::vk;

use super::*;

fn offset3(x: i32, y: i32, z: i32) -> vk::Offset3D {
    vk::Offset3D { x, y, z }
}

impl State {
    /// The colour attachments and the depth attachment of `target`.
    pub(super) fn attachments(&self, target: TargetRef) -> Option<(Vec<Attachment>, Option<Attachment>)> {
        match target {
            TargetRef::Window => {
                let w = self.window.as_ref()?;
                let attachment = |id: RawId| {
                    self.textures.get(&id).map(|t| Attachment { texture: id, desc: t.desc, mip: 0, layer: None })
                };
                Some((vec![attachment(w.color)?], attachment(w.depth)))
            }
            TargetRef::Target(id) => {
                let (colors, depth) = self.targets.get(&id)?;
                // The texture's own description (the backend may have
                // changed the sample count).
                let fix = |a: &Attachment| self.textures.get(&a.texture).map(|t| Attachment { desc: t.desc, ..*a });
                Some((colors.iter().filter_map(fix).collect(), depth.as_ref().and_then(fix)))
            }
        }
    }

    fn mip_size(desc: &TextureDesc, mip: u32) -> (u32, u32) {
        ((desc.width >> mip).max(1), (desc.height >> mip).max(1))
    }

    pub(super) fn begin_pass(&mut self, target: TargetRef, size: (u32, u32), color: Load, depth: Load) {
        if target == TargetRef::Window {
            self.update_window_target();
        }
        self.interrupt_rendering();
        self.pass = Some(Pass { target, color, depth });
        self.viewport = Rect::sized(size.0, size.1);
        self.scissor = None;
        self.dynamic_dirty = true;
    }

    pub(super) fn end_pass(&mut self) {
        self.interrupt_rendering();
        self.pass = None;
    }

    /// End rendering (doing the clears a pass begun asked for first), for
    /// commands that cannot run inside it.
    pub(super) fn interrupt_rendering(&mut self) {
        if !self.rendering {
            let clears = self
                .pass
                .is_some_and(|p| matches!(p.color, Load::Color(_)) || matches!(p.depth, Load::Depth(_)));
            if clears {
                self.begin_rendering(&[]);
            }
        }
        if self.rendering {
            unsafe { self.gpu.device.cmd_end_rendering(self.cmd()) };
            self.rendering = false;
        }
    }

    /// Be rendering into the pass's target, with `sampled` readable by
    /// shaders.
    pub(super) fn ensure_rendering(&mut self, sampled: &[RawId]) -> bool {
        if self.rendering {
            let ready = sampled.iter().all(|id| {
                self.textures
                    .get(id)
                    .is_none_or(|t| t.layouts.iter().all(|&l| l == vk::ImageLayout::READ_ONLY_OPTIMAL))
            });
            if ready {
                return true;
            }
            unsafe { self.gpu.device.cmd_end_rendering(self.cmd()) };
            self.rendering = false;
        }
        self.begin_rendering(sampled)
    }

    fn begin_rendering(&mut self, sampled: &[RawId]) -> bool {
        let Some(pass) = self.pass else {
            return false;
        };
        let Some((colors, depth)) = self.attachments(pass.target) else {
            return false;
        };
        let cmd = self.cmd();
        let mut list = Vec::new();
        for a in colors.iter().chain(depth.iter()) {
            let layers = match a.layer {
                Some(l) => l..l + 1,
                None => 0..u32::MAX,
            };
            list.push((a.texture, a.mip..a.mip + 1, layers, vk::ImageLayout::ATTACHMENT_OPTIMAL));
        }
        for &id in sampled {
            list.push((id, 0..u32::MAX, 0..u32::MAX, vk::ImageLayout::READ_ONLY_OPTIMAL));
        }
        self.transition(cmd, &list);

        let mut extent = (u32::MAX, u32::MAX);
        let mut layer_count = u32::MAX;
        let mut samples = 1;
        for a in colors.iter().chain(depth.iter()) {
            let (w, h) = Self::mip_size(&a.desc, a.mip);
            extent = (extent.0.min(w), extent.1.min(h));
            let layers = match (a.layer, a.desc.kind) {
                (None, TextureKind::D2Array | TextureKind::CubeArray) => a.desc.layers.max(1),
                _ => 1,
            };
            layer_count = layer_count.min(layers);
            samples = a.desc.samples.max(1);
        }
        if extent.0 == u32::MAX {
            return false;
        }
        let color_load = |a: Load| match a {
            Load::Color(c) => (vk::AttachmentLoadOp::CLEAR, vk::ClearValue { color: vk::ClearColorValue { float32: c } }),
            _ => (vk::AttachmentLoadOp::LOAD, vk::ClearValue::default()),
        };
        let (load, clear) = color_load(pass.color);
        let mut color_infos = Vec::new();
        let mut formats: ColorFormats = ([vk::Format::UNDEFINED; MAX_COLORS], 0);
        for a in colors.iter().take(MAX_COLORS) {
            let view = self.attachment_view(a.texture, a.mip, a.layer);
            formats.0[formats.1 as usize] = self.textures[&a.texture].format;
            formats.1 += 1;
            color_infos.push(
                vk::RenderingAttachmentInfo::default()
                    .image_view(view)
                    .image_layout(vk::ImageLayout::ATTACHMENT_OPTIMAL)
                    .load_op(load)
                    .store_op(vk::AttachmentStoreOp::STORE)
                    .clear_value(clear),
            );
        }
        let (depth_load, depth_clear) = match pass.depth {
            Load::Depth(d) => (
                vk::AttachmentLoadOp::CLEAR,
                vk::ClearValue { depth_stencil: vk::ClearDepthStencilValue { depth: d, stencil: 0 } },
            ),
            _ => (vk::AttachmentLoadOp::LOAD, vk::ClearValue::default()),
        };
        let mut depth_format = vk::Format::UNDEFINED;
        let mut stencil = false;
        let depth_info = depth.map(|a| {
            let view = self.attachment_view(a.texture, a.mip, a.layer);
            depth_format = self.textures[&a.texture].format;
            stencil = a.desc.format.has_stencil();
            vk::RenderingAttachmentInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::ATTACHMENT_OPTIMAL)
                .load_op(depth_load)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(depth_clear)
        });
        let mut info = vk::RenderingInfo::default()
            .render_area(vk::Rect2D { offset: vk::Offset2D::default(), extent: vk::Extent2D { width: extent.0, height: extent.1 } })
            .layer_count(layer_count)
            .color_attachments(&color_infos);
        if let Some(d) = depth_info.as_ref() {
            info = info.depth_attachment(d);
            if stencil {
                info = info.stencil_attachment(d);
            }
        }
        unsafe { self.gpu.device.cmd_begin_rendering(cmd, &info) };
        if let Some(p) = self.pass.as_mut() {
            p.color = Load::Keep;
            p.depth = Load::Keep;
        }
        self.rendering = true;
        self.rendering_key = (formats, depth_format, stencil, samples);
        self.render_extent = extent;
        self.render_layers = layer_count;
        self.dynamic_dirty = true;
        true
    }

    /// Whether `texture` is an attachment of the current pass's target.
    pub(super) fn is_attached(&self, texture: RawId) -> bool {
        self.pass
            .and_then(|p| match p.target {
                TargetRef::Window => self.window.as_ref().map(|w| texture == w.color || texture == w.depth),
                TargetRef::Target(id) => self
                    .targets
                    .get(&id)
                    .map(|(c, d)| c.iter().chain(d.iter()).any(|a| a.texture == texture)),
            })
            .unwrap_or(false)
    }

    /// The scissor (or the whole target) within the target.
    fn scissor_rect(&self) -> vk::Rect2D {
        let (w, h) = self.render_extent;
        let r = self.scissor.unwrap_or(Rect::sized(w, h));
        let x0 = r.x.clamp(0, w as i32);
        let y0 = r.y.clamp(0, h as i32);
        let x1 = (r.x + r.width as i32).clamp(x0, w as i32);
        let y1 = (r.y + r.height as i32).clamp(y0, h as i32);
        vk::Rect2D {
            offset: vk::Offset2D { x: x0, y: y0 },
            extent: vk::Extent2D { width: (x1 - x0) as u32, height: (y1 - y0) as u32 },
        }
    }

    fn apply_dynamic_state(&mut self) {
        if !self.dynamic_dirty {
            return;
        }
        self.dynamic_dirty = false;
        let cmd = self.cmd();
        let v = self.viewport;
        let viewport = vk::Viewport {
            x: v.x as f32,
            y: v.y as f32,
            width: v.width.max(1) as f32,
            height: v.height.max(1) as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let scissor = self.scissor_rect();
        unsafe {
            let d = &self.gpu.device;
            d.cmd_set_viewport(cmd, 0, &[viewport]);
            d.cmd_set_scissor(cmd, 0, &[scissor]);
            d.cmd_set_stencil_reference(cmd, vk::StencilFaceFlags::FRONT_AND_BACK, self.stencil_reference as u32);
        }
    }

    pub(super) fn draw(&mut self, indexed: Option<i32>, first: u32, count: u32, first_instance: u32, instances: u32) {
        let Some(entry) = self.pipeline.and_then(|p| self.pipelines.get(&p)) else {
            return;
        };
        let (program, vao, layouts, missing) = (entry.program, entry.vao, entry.desc.vertex_layouts, entry.missing);
        let Some(desc) = self.resolved else {
            return;
        };
        if self.pass.is_none() {
            self.warn_once("a draw outside any pass is skipped".into());
            return;
        }
        if !self.programs.contains_key(&program) {
            return;
        }
        let mut images = std::mem::take(&mut self.scratch_images);
        self.sampler_bindings(program, &mut images);
        let mut sampled = std::mem::take(&mut self.scratch_sampled);
        sampled.clear();
        sampled.extend(images.iter().map(|i| i.3));
        sampled.sort_unstable();
        sampled.dedup();
        let ready = self.ensure_rendering(&sampled);
        self.scratch_sampled = sampled;
        if !ready {
            self.scratch_images = images;
            return;
        }
        let line = self.line;
        let Some(pipeline) = self.vk_pipeline(program, &desc, line) else {
            self.scratch_images = images;
            return;
        };
        let cmd = self.cmd();
        if pipeline != self.bound_pipeline {
            unsafe { self.gpu.device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline) };
            self.bound_pipeline = pipeline;
        }
        self.apply_dynamic_state();

        // Vertex buffers, by slot; a slot nothing is bound to reads zeros.
        let vertex = self.vaos[vao].vertex;
        let index_id = self.vaos[vao].index;
        let mut buffers = [vk::Buffer::null(); Layouts::MAX + 1];
        let mut offsets = [0u64; Layouts::MAX + 1];
        let mut strides = [0u64; Layouts::MAX + 1];
        let mut n = 0;
        for slot in 0..layouts.len() {
            let bound = vertex.get(slot).copied().flatten();
            let found = bound.and_then(|(id, offset, stride)| self.use_buffer(id).map(|(b, _)| (b, offset, stride)));
            let (b, o, s) = match found {
                Some((b, o, s)) => (b, o as u64, s as u64),
                None => (self.use_buffer(self.dummy_buffer).map_or(vk::Buffer::null(), |b| b.0), 0, 0),
            };
            buffers[n] = b;
            offsets[n] = o;
            strides[n] = s;
            n += 1;
        }
        if missing {
            buffers[n] = self.use_buffer(self.default_attribute).map_or(vk::Buffer::null(), |b| b.0);
            n += 1;
        }
        if n > 0 {
            unsafe {
                self.gpu.device.cmd_bind_vertex_buffers2(cmd, 0, &buffers[..n], &offsets[..n], None, Some(&strides[..n]));
            }
        }
        let pushed = self.push_descriptors(program, &images) && self.bind_params(program);
        self.scratch_images = images;
        if !pushed {
            return;
        }
        unsafe {
            match indexed {
                Some(base_vertex) => {
                    let Some(index) = index_id.and_then(|id| self.use_buffer(id)) else {
                        self.warn_once("an indexed draw without an index buffer is skipped".into());
                        return;
                    };
                    self.gpu.device.cmd_bind_index_buffer(cmd, index.0, 0, self.index_type);
                    self.gpu.device.cmd_draw_indexed(cmd, count, instances, first, base_vertex, first_instance);
                }
                None => self.gpu.device.cmd_draw(cmd, count, instances, first, first_instance),
            }
        }
    }

    pub(super) fn clear(&mut self, planes: Planes, color: [f32; 4], depth: f32, stencil: u8) {
        let Some(pass) = self.pass else {
            return;
        };
        let Some((colors, depth_attachment)) = self.attachments(pass.target) else {
            return;
        };
        let has_stencil = depth_attachment.is_some_and(|d| d.desc.format.has_stencil());
        let want_color = planes.color && !colors.is_empty();
        let want_depth = planes.depth && depth_attachment.is_some();
        let want_stencil = planes.stencil && has_stencil;
        if !want_color && !want_depth && !want_stencil {
            return;
        }
        // Not rendering yet and all of it: the clear is how rendering begins.
        if !self.rendering && self.scissor.is_none() {
            let depth_foldable = !want_depth && !want_stencil
                || (want_depth && (want_stencil || !has_stencil) && stencil == 0);
            if depth_foldable {
                if let Some(p) = self.pass.as_mut() {
                    if want_color {
                        p.color = Load::Color(color);
                    }
                    if want_depth {
                        p.depth = Load::Depth(depth);
                    }
                }
                return;
            }
        }
        if !self.ensure_rendering(&[]) {
            return;
        }
        let rect = self.scissor_rect();
        if rect.extent.width == 0 || rect.extent.height == 0 {
            return;
        }
        let mut clears = Vec::new();
        if want_color {
            for i in 0..colors.len() {
                clears.push(vk::ClearAttachment {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    color_attachment: i as u32,
                    clear_value: vk::ClearValue { color: vk::ClearColorValue { float32: color } },
                });
            }
        }
        let mut aspect = vk::ImageAspectFlags::empty();
        if want_depth {
            aspect |= vk::ImageAspectFlags::DEPTH;
        }
        if want_stencil {
            aspect |= vk::ImageAspectFlags::STENCIL;
        }
        if !aspect.is_empty() {
            clears.push(vk::ClearAttachment {
                aspect_mask: aspect,
                color_attachment: 0,
                clear_value: vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue { depth, stencil: stencil as u32 },
                },
            });
        }
        let rects = [vk::ClearRect { rect, base_array_layer: 0, layer_count: self.render_layers }];
        unsafe { self.gpu.device.cmd_clear_attachments(self.cmd(), &clears, &rects) };
    }

    fn layers_of(a: &Attachment, aspect: vk::ImageAspectFlags) -> vk::ImageSubresourceLayers {
        vk::ImageSubresourceLayers {
            aspect_mask: aspect,
            mip_level: a.mip,
            base_array_layer: a.layer.unwrap_or(0),
            layer_count: 1,
        }
    }

    fn sub(a: &Attachment) -> (std::ops::Range<u32>, std::ops::Range<u32>) {
        let l = a.layer.unwrap_or(0);
        (a.mip..a.mip + 1, l..l + 1)
    }

    pub(super) fn blit(&mut self, src: TargetRef, src_rect: Rect, dst: TargetRef, dst_rect: Rect, planes: Planes, linear: bool) {
        let (Some((src_colors, src_depth)), Some((dst_colors, dst_depth))) = (self.attachments(src), self.attachments(dst))
        else {
            return;
        };
        self.interrupt_rendering();
        if planes.color {
            if let Some(s) = src_colors.first() {
                for d in &dst_colors {
                    self.blit_color(s, src_rect, d, dst_rect, linear);
                }
            }
        }
        if planes.depth || planes.stencil {
            if let (Some(s), Some(d)) = (src_depth, dst_depth) {
                self.blit_depth(&s, src_rect, &d, dst_rect);
            }
        }
    }

    fn blit_color(&mut self, s: &Attachment, sr: Rect, d: &Attachment, dr: Rect, linear: bool) {
        let (Some(st), Some(dt)) = (self.textures.get(&s.texture), self.textures.get(&d.texture)) else {
            return;
        };
        let (s_image, s_format, s_samples) = (st.image, st.format, st.desc.samples);
        let (d_image, d_format, d_samples) = (dt.image, dt.format, dt.desc.samples);
        let cmd = self.cmd();
        let same_size = sr.width == dr.width && sr.height == dr.height;
        let (sm, sl) = Self::sub(s);
        let (dm, dl) = Self::sub(d);
        let color = vk::ImageAspectFlags::COLOR;
        if s_samples > 1 && d_samples == 1 {
            if s_format == d_format && same_size {
                self.transition(
                    cmd,
                    &[
                        (s.texture, sm, sl, vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                        (d.texture, dm, dl, vk::ImageLayout::TRANSFER_DST_OPTIMAL),
                    ],
                );
                let region = vk::ImageResolve {
                    src_subresource: Self::layers_of(s, color),
                    src_offset: offset3(sr.x, sr.y, 0),
                    dst_subresource: Self::layers_of(d, color),
                    dst_offset: offset3(dr.x, dr.y, 0),
                    extent: vk::Extent3D { width: sr.width, height: sr.height, depth: 1 },
                };
                unsafe {
                    self.gpu.device.cmd_resolve_image(
                        cmd,
                        s_image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        d_image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &[region],
                    );
                }
            } else {
                // Resolve into a texture like the source, then blit that.
                let temp_desc = TextureDesc::d2(st.desc.format, sr.width.max(1), sr.height.max(1));
                let temp = self.create_texture(&temp_desc);
                let ta = Attachment { texture: temp, desc: temp_desc, mip: 0, layer: None };
                self.blit_color(s, sr, &ta, Rect::sized(sr.width, sr.height), false);
                self.blit_color(&ta, Rect::sized(sr.width, sr.height), d, dr, linear);
                self.destroy(Raw::Texture(temp));
            }
            return;
        }
        if s_samples != d_samples {
            self.warn_once(format!("blit from {s_samples} to {d_samples} samples is not supported"));
            return;
        }
        if s_samples > 1 {
            if !same_size || s_format != d_format {
                self.warn_once("blit between multisampled targets of other sizes or formats".into());
                return;
            }
            self.transition(
                cmd,
                &[
                    (s.texture, sm, sl, vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                    (d.texture, dm, dl, vk::ImageLayout::TRANSFER_DST_OPTIMAL),
                ],
            );
            let region = vk::ImageCopy {
                src_subresource: Self::layers_of(s, color),
                src_offset: offset3(sr.x, sr.y, 0),
                dst_subresource: Self::layers_of(d, color),
                dst_offset: offset3(dr.x, dr.y, 0),
                extent: vk::Extent3D { width: sr.width, height: sr.height, depth: 1 },
            };
            unsafe {
                self.gpu.device.cmd_copy_image(
                    cmd,
                    s_image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    d_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            }
            return;
        }
        if s.texture == d.texture && sm == dm && sl == dl {
            self.warn_once("blit within one image".into());
            return;
        }
        self.transition(
            cmd,
            &[
                (s.texture, sm, sl, vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                (d.texture, dm, dl, vk::ImageLayout::TRANSFER_DST_OPTIMAL),
            ],
        );
        let region = vk::ImageBlit {
            src_subresource: Self::layers_of(s, color),
            src_offsets: [offset3(sr.x, sr.y, 0), offset3(sr.x + sr.width as i32, sr.y + sr.height as i32, 1)],
            dst_subresource: Self::layers_of(d, color),
            dst_offsets: [offset3(dr.x, dr.y, 0), offset3(dr.x + dr.width as i32, dr.y + dr.height as i32, 1)],
        };
        let filter = if linear { vk::Filter::LINEAR } else { vk::Filter::NEAREST };
        unsafe {
            self.gpu.device.cmd_blit_image(
                cmd,
                s_image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                d_image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
                filter,
            );
        }
    }

    /// Depth (and stencil) from `s` to `d`. A multisampled source is
    /// resolved by a render pass that only resolves (sample 0, as GL's
    /// NEAREST blit picks one sample): copies and blits cannot read
    /// multisampled depth.
    fn blit_depth(&mut self, s: &Attachment, sr: Rect, d: &Attachment, dr: Rect) {
        let (Some(st), Some(dt)) = (self.textures.get(&s.texture), self.textures.get(&d.texture)) else {
            return;
        };
        let (s_image, s_format, s_samples, s_desc) = (st.image, st.format, st.desc.samples, st.desc);
        let (d_image, d_format, d_samples) = (dt.image, dt.format, dt.desc.samples);
        if s_format != d_format {
            self.warn_once(format!("depth blit between formats {s_format:?} and {d_format:?}"));
            return;
        }
        let cmd = self.cmd();
        let same_size = sr.width == dr.width && sr.height == dr.height;
        let (sm, sl) = Self::sub(s);
        let (dm, dl) = Self::sub(d);
        let aspect = convert::aspects(s_desc.format);
        if s_samples > 1 && d_samples == 1 {
            if sr != dr {
                self.warn_once("multisampled depth blit to another place".into());
                return;
            }
            self.transition(
                cmd,
                &[
                    (s.texture, sm, sl, vk::ImageLayout::ATTACHMENT_OPTIMAL),
                    (d.texture, dm, dl, vk::ImageLayout::ATTACHMENT_OPTIMAL),
                ],
            );
            let src_view = self.attachment_view(s.texture, s.mip, s.layer);
            let dst_view = self.attachment_view(d.texture, d.mip, d.layer);
            let info = vk::RenderingAttachmentInfo::default()
                .image_view(src_view)
                .image_layout(vk::ImageLayout::ATTACHMENT_OPTIMAL)
                .resolve_mode(vk::ResolveModeFlags::SAMPLE_ZERO)
                .resolve_image_view(dst_view)
                .resolve_image_layout(vk::ImageLayout::ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE);
            let mut rendering = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D { x: sr.x, y: sr.y },
                    extent: vk::Extent2D { width: sr.width, height: sr.height },
                })
                .layer_count(1)
                .depth_attachment(&info);
            if s_desc.format.has_stencil() {
                rendering = rendering.stencil_attachment(&info);
            }
            unsafe {
                self.gpu.device.cmd_begin_rendering(cmd, &rendering);
                self.gpu.device.cmd_end_rendering(cmd);
            }
            return;
        }
        if s_samples != d_samples {
            self.warn_once("depth blit into a multisampled target".into());
            return;
        }
        self.transition(
            cmd,
            &[
                (s.texture, sm, sl, vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
                (d.texture, dm, dl, vk::ImageLayout::TRANSFER_DST_OPTIMAL),
            ],
        );
        unsafe {
            if same_size {
                let region = vk::ImageCopy {
                    src_subresource: Self::layers_of(s, aspect),
                    src_offset: offset3(sr.x, sr.y, 0),
                    dst_subresource: Self::layers_of(d, aspect),
                    dst_offset: offset3(dr.x, dr.y, 0),
                    extent: vk::Extent3D { width: sr.width, height: sr.height, depth: 1 },
                };
                self.gpu.device.cmd_copy_image(
                    cmd,
                    s_image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    d_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
            } else if s_samples == 1 {
                let region = vk::ImageBlit {
                    src_subresource: Self::layers_of(s, aspect),
                    src_offsets: [offset3(sr.x, sr.y, 0), offset3(sr.x + sr.width as i32, sr.y + sr.height as i32, 1)],
                    dst_subresource: Self::layers_of(d, aspect),
                    dst_offsets: [offset3(dr.x, dr.y, 0), offset3(dr.x + dr.width as i32, dr.y + dr.height as i32, 1)],
                };
                self.gpu.device.cmd_blit_image(
                    cmd,
                    s_image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    d_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                    vk::Filter::NEAREST,
                );
            }
        }
    }

    pub(super) fn read_pixels(&mut self, target: TargetRef, rect: Rect) -> Vec<u8> {
        let size = rect.width as usize * rect.height as usize * 4;
        let Some(pending) = self.copy_pixels_out(target, rect) else {
            return vec![0u8; size];
        };
        self.flush_and_wait();
        self.finish_read(pending)
    }

    /// `read_pixels` without waiting: the copy goes into this frame's
    /// commands; `take_pixels` hands the pixels out once the GPU ran them.
    pub(super) fn read_pixels_later(&mut self, target: TargetRef, rect: Rect) -> u64 {
        let id = self.id();
        let size = rect.width as usize * rect.height as usize * 4;
        let read = match self.copy_pixels_out(target, rect) {
            Some(pending) => Readback::Pending(pending),
            None => Readback::Ready(vec![0u8; size]),
        };
        self.readbacks.insert(id, read);
        id
    }

    /// The pixels of a `read_pixels_later`, once the GPU is done with it.
    pub(super) fn take_pixels(&mut self, id: u64) -> Option<Vec<u8>> {
        self.poll_completed();
        let ready = match self.readbacks.get(&id)? {
            Readback::Ready(_) => true,
            Readback::Pending(p) => p.serial <= self.completed,
        };
        if !ready {
            return None;
        }
        match self.readbacks.remove(&id)? {
            Readback::Ready(pixels) => Some(pixels),
            Readback::Pending(pending) => Some(self.finish_read(pending)),
        }
    }

    /// Record copying `rect` of `target`'s first colour attachment into
    /// host memory (resolving and converting to RGBA8 on the way).
    fn copy_pixels_out(&mut self, target: TargetRef, rect: Rect) -> Option<PendingRead> {
        let size = rect.width as usize * rect.height as usize * 4;
        let (colors, _) = self.attachments(target)?;
        let source = colors.first().copied()?;
        if rect.width == 0 || rect.height == 0 {
            return None;
        }
        self.interrupt_rendering();
        let temp_desc = TextureDesc::d2(Format::Rgba8, rect.width, rect.height);
        let temp = self.create_texture(&temp_desc);
        let temp_attachment = Attachment { texture: temp, desc: temp_desc, mip: 0, layer: None };
        self.blit_color(&source, rect, &temp_attachment, Rect::sized(rect.width, rect.height), false);
        let Some((buffer, allocation)) = self.raw_buffer(
            size as u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            gpu_allocator::MemoryLocation::GpuToCpu,
            "readback",
        ) else {
            self.destroy(Raw::Texture(temp));
            return None;
        };
        let cmd = self.cmd();
        self.transition_all(cmd, &[temp], vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
        let region = vk::BufferImageCopy {
            buffer_offset: 0,
            buffer_row_length: 0,
            buffer_image_height: 0,
            image_subresource: vk::ImageSubresourceLayers {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                mip_level: 0,
                base_array_layer: 0,
                layer_count: 1,
            },
            image_offset: vk::Offset3D::default(),
            image_extent: vk::Extent3D { width: rect.width, height: rect.height, depth: 1 },
        };
        let image = self.textures[&temp].image;
        unsafe {
            self.gpu.device.cmd_copy_image_to_buffer(cmd, image, vk::ImageLayout::TRANSFER_SRC_OPTIMAL, buffer, &[region]);
        }
        self.global_barrier(cmd);
        // Freed once the commands recorded now have run.
        self.destroy(Raw::Texture(temp));
        Some(PendingRead { buffer, allocation, serial: self.serial, size })
    }

    /// The pixels of a copy the GPU has finished; frees its buffer.
    fn finish_read(&mut self, pending: PendingRead) -> Vec<u8> {
        let mut pixels = vec![0u8; pending.size];
        if let Some(ptr) = pending.allocation.mapped_ptr() {
            unsafe { std::ptr::copy_nonoverlapping(ptr.as_ptr() as *const u8, pixels.as_mut_ptr(), pending.size) };
        }
        unsafe { self.gpu.device.destroy_buffer(pending.buffer, None) };
        let _ = self.allocator.free(pending.allocation);
        pixels
    }

    fn recreate_swapchain(&mut self) {
        let size = self.window.as_ref().map_or((1, 1), |w| w.size);
        unsafe {
            let _ = self.gpu.device.device_wait_idle();
        }
        let old = self.swapchain.take();
        let old_handle = old.as_ref().map_or(vk::SwapchainKHR::null(), |s| s.handle);
        match context::create_swapchain(&self.gpu, old_handle, size, self.vsync) {
            Ok(s) => self.swapchain = Some(s),
            Err(e) => log::error!("Vulkan: {e}"),
        }
        if let Some(old) = old {
            context::destroy_swapchain(&self.gpu, &old);
        }
        self.swapchain_dirty = false;
    }

    pub(super) fn end_frame(&mut self) {
        self.interrupt_rendering();
        self.pass = None;
        let window_size = self.window.as_ref().map_or((1, 1), |w| w.size);
        let stale = self
            .swapchain
            .as_ref()
            .is_none_or(|s| (s.extent.width, s.extent.height) != window_size);
        if self.swapchain_dirty || stale {
            self.recreate_swapchain();
        }
        let acquired = self.frames[self.frame].acquired;
        let image = self.swapchain.as_ref().and_then(|s| {
            match unsafe { self.gpu.swapchain_fn.acquire_next_image(s.handle, u64::MAX, acquired, vk::Fence::null()) } {
                Ok((index, suboptimal)) => Some((index, suboptimal)),
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => None,
                Err(e) => {
                    log::error!("Vulkan: acquiring the swapchain image: {e}");
                    None
                }
            }
        });
        let Some((index, suboptimal)) = image else {
            self.swapchain_dirty = true;
            self.submit(vk::Semaphore::null(), vk::Semaphore::null());
            self.frame = (self.frame + 1) % FRAMES;
            self.recycle_frame();
            return;
        };
        if suboptimal {
            self.swapchain_dirty = true;
        }
        let (swap_image, extent, finished, handle) = {
            let s = self.swapchain.as_ref().expect("acquired from it");
            (s.images[index as usize], s.extent, s.finished[index as usize], s.handle)
        };
        // Resolve the window's samples, then copy upside down (its row 0
        // is the bottom) into the swapchain image.
        if let Some(w) = self.window.as_ref() {
            let (color, resolve, size) = (w.color, w.resolve, w.size);
            let rect = Rect::sized(size.0, size.1);
            let a = |id: RawId, s: &State| Attachment { texture: id, desc: s.textures[&id].desc, mip: 0, layer: None };
            let (ca, ra) = (a(color, self), a(resolve, self));
            self.blit_color(&ca, rect, &ra, rect, false);
            let cmd = self.cmd();
            self.transition_all(cmd, &[resolve], vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
            let range = vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            };
            let to_dst = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .src_access_mask(vk::AccessFlags2::empty())
                .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .image(swap_image)
                .subresource_range(range)];
            let to_present = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
                .dst_access_mask(vk::AccessFlags2::empty())
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
                .image(swap_image)
                .subresource_range(range)];
            let (w, h) = (size.0 as i32, size.1 as i32);
            let blit = vk::ImageBlit {
                src_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                src_offsets: [offset3(0, h, 0), offset3(w, 0, 1)],
                dst_subresource: vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                },
                dst_offsets: [offset3(0, 0, 0), offset3(extent.width as i32, extent.height as i32, 1)],
            };
            let filter = if (extent.width, extent.height) == size { vk::Filter::NEAREST } else { vk::Filter::LINEAR };
            let resolve_image = self.textures[&resolve].image;
            unsafe {
                let d = &self.gpu.device;
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_dst));
                d.cmd_blit_image(
                    cmd,
                    resolve_image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    swap_image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[blit],
                    filter,
                );
                d.cmd_pipeline_barrier2(cmd, &vk::DependencyInfo::default().image_memory_barriers(&to_present));
            }
        }
        self.submit(acquired, finished);
        let waits = [finished];
        let swapchains = [handle];
        let indices = [index];
        let present = vk::PresentInfoKHR::default().wait_semaphores(&waits).swapchains(&swapchains).image_indices(&indices);
        match unsafe { self.gpu.swapchain_fn.queue_present(self.gpu.queue, &present) } {
            Ok(false) => {}
            Ok(true) | Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.swapchain_dirty = true,
            Err(e) => log::error!("Vulkan: present: {e}"),
        }
        self.frame = (self.frame + 1) % FRAMES;
        self.recycle_frame();
    }
}
