//! Motion vectors of moving models.
//!
//! The camera's motion vectors (`velocity.glsl`) treat everything as if it
//! stood still in the world. A car driving along with the camera, or a
//! walking character, moves differently: TAA would smear it. So while the
//! camera's view is drawn, models record each mesh that moved since the
//! frame before (`record`): its transform and bones now and then. Once the
//! view is resolved these meshes are drawn again into the motion vectors,
//! over the camera's, where they are seen (`draw`).
//!
//! What a model remembers between frames is kept per mesh in `History`:
//! a mesh drawn more than once in a view (the same model at two places)
//! has no single previous position and keeps the camera's motion.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use cgmath::Matrix4;

use crate::core::{
    gfx::{self, RawId},
    renderer::shader::Shader,
};

/// As `model/vertex.glsl`.
pub const MAX_BONES: usize = 100;

/// A mesh to draw into the motion vectors.
struct MovingDraw {
    vertices: RawId,
    indices: Option<RawId>,
    count: u32,
    stride: u32,
    layout: gfx::VertexLayout,
    two_sided: bool,
    model: Matrix4<f32>,
    previous_model: Matrix4<f32>,
    /// Skinning matrices now and a frame before; `None` for a mesh without
    /// bones.
    bones: Option<(Vec<Matrix4<f32>>, Vec<Matrix4<f32>>)>,
}

thread_local! {
    /// Models record while the camera's view is drawn (`begin_frame`
    /// until `end_frame`).
    static RECORDING: Cell<bool> = const { Cell::new(false) };
    /// Counts views drawn with motion vectors.
    static FRAME: Cell<u64> = const { Cell::new(0) };
    static DRAWS: RefCell<Vec<MovingDraw>> = const { RefCell::new(Vec::new()) };
    static SHADER: RefCell<Option<Shader>> = const { RefCell::new(None) };
}

/// Start recording for the camera's view about to be drawn.
pub fn begin_frame() {
    FRAME.with(|f| f.set(f.get() + 1));
    DRAWS.with(|d| d.borrow_mut().clear());
    RECORDING.with(|r| r.set(true));
}

/// Stop recording (the view is drawn).
pub fn end_frame() {
    RECORDING.with(|r| r.set(false));
}

/// Whether models record now: the camera's view is drawn, with motion
/// vectors.
pub fn recording() -> bool {
    RECORDING.with(Cell::get)
}

fn frame() -> u64 {
    FRAME.with(Cell::get)
}

/// What a model remembers of each of its meshes from the last view.
#[derive(Default)]
pub struct History {
    meshes: RefCell<HashMap<usize, Seen>>,
}

#[derive(Clone)]
struct Seen {
    frame: u64,
    /// Drawn more than once in that view: no single place.
    ambiguous: bool,
    model: Matrix4<f32>,
    bones: Option<Vec<Matrix4<f32>>>,
}

/// One mesh of a model, as drawn now.
pub struct MeshDraw<'a> {
    pub index: usize,
    pub vertices: RawId,
    pub indices: Option<RawId>,
    pub count: u32,
    pub layout: gfx::VertexLayout,
    pub two_sided: bool,
    pub model: Matrix4<f32>,
    pub bones: Option<&'a [Matrix4<f32>]>,
}

impl History {
    /// Remember `draw` for the next view, and if it moved since the last
    /// one, draw it into the motion vectors later. Call only while
    /// `recording`.
    pub fn record(&self, draw: MeshDraw) {
        let now = frame();
        let mut meshes = self.meshes.borrow_mut();
        let seen = meshes.get(&draw.index).cloned();
        if let Some(seen) = &seen {
            if seen.frame == now {
                // A second time in this view.
                meshes.get_mut(&draw.index).expect("just read").ambiguous = true;
                return;
            }
        }
        meshes.insert(
            draw.index,
            Seen {
                frame: now,
                ambiguous: false,
                model: draw.model,
                bones: draw.bones.map(<[_]>::to_vec),
            },
        );
        let Some(seen) = seen.filter(|seen| seen.frame + 1 == now && !seen.ambiguous) else {
            return;
        };
        let bones = match (draw.bones, seen.bones) {
            (Some(now), Some(before)) if before.len() == now.len() => Some((now.to_vec(), before)),
            (None, None) => None,
            // The skeleton changed shape: no motion to tell.
            _ => return,
        };
        let moved = seen.model != draw.model || bones.as_ref().is_some_and(|(now, before)| now != before);
        if !moved {
            return;
        }
        DRAWS.with(|d| {
            d.borrow_mut().push(MovingDraw {
                vertices: draw.vertices,
                indices: draw.indices,
                count: draw.count,
                stride: draw.layout.stride,
                layout: draw.layout,
                two_sided: draw.two_sided,
                model: draw.model,
                previous_model: seen.model,
                bones,
            })
        });
    }
}

/// How many meshes `draw` would draw (for tests and statistics).
pub fn pending() -> usize {
    DRAWS.with(|d| d.borrow().len())
}

/// Draw the meshes recorded into the motion vectors' pass running now:
/// `view_projection` is what the view was drawn with (jittered), `current`
/// and `previous` the cameras without jitter.
pub fn draw(view_projection: &Matrix4<f32>, current: &Matrix4<f32>, previous: &Matrix4<f32>) {
    let draws = DRAWS.with(|d| std::mem::take(&mut *d.borrow_mut()));
    if draws.is_empty() {
        return;
    }
    SHADER.with(|shader| {
        let mut shader = shader.borrow_mut();
        let shader = shader.get_or_insert_with(|| {
            Shader::new(include_str!("motion_vertex.glsl"), include_str!("motion_fragment.glsl"))
        });
        let identity = [Matrix4::from_scale(1.0)];
        let mut current_pipeline: Option<(u32, bool)> = None;
        for draw in &draws {
            if current_pipeline != Some((draw.stride, draw.two_sided)) {
                current_pipeline = Some((draw.stride, draw.two_sided));
                let desc = gfx::PipelineDesc {
                    vertex_layouts: gfx::Layouts::of(&[draw.layout.clone()]),
                    depth: gfx::DepthState::LEQUAL.write(false),
                    cull: if draw.two_sided {
                        gfx::Cull::None
                    } else {
                        gfx::Cull::Back
                    },
                    ..Default::default()
                };
                gfx::set_pipeline(shader.pipeline(&desc));
                shader.set_uniform_mat4("viewProjection", view_projection);
                shader.set_uniform_mat4("currentCamera", current);
                shader.set_uniform_mat4("previousCamera", previous);
            }
            shader.set_uniform_mat4("model", &draw.model);
            shader.set_uniform_mat4("previousModel", &draw.previous_model);
            match &draw.bones {
                Some((now, before)) => {
                    shader.set_uniform_mat4_array("boneTransforms", now);
                    shader.set_uniform_mat4_array("previousBones", before);
                }
                None => {
                    shader.set_uniform_mat4_array("boneTransforms", &identity);
                    shader.set_uniform_mat4_array("previousBones", &identity);
                }
            }
            gfx::set_raw_vertex_buffer(0, draw.vertices, 0, draw.stride);
            match draw.indices {
                Some(indices) => {
                    gfx::set_raw_index_buffer(indices, gfx::IndexFormat::U32);
                    gfx::draw_indexed(0, draw.count, 0);
                }
                None => gfx::draw(0, draw.count),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use cgmath::Vector3;

    fn mesh(model: Matrix4<f32>) -> MeshDraw<'static> {
        MeshDraw {
            index: 0,
            vertices: 1,
            indices: None,
            count: 3,
            layout: gfx::VertexLayout::packed(0, gfx::StepMode::Vertex, &[gfx::VertexFormat::F32x3]),
            two_sided: false,
            model,
            bones: None,
        }
    }

    #[test]
    fn only_meshes_that_moved_since_the_last_view_are_drawn() {
        let history = History::default();
        let still = Matrix4::from_translation(Vector3::new(1.0, 2.0, 3.0));
        begin_frame();
        history.record(mesh(still));
        assert_eq!(pending(), 0, "nothing to compare with yet");
        begin_frame();
        history.record(mesh(still));
        assert_eq!(pending(), 0, "it stood still");
        begin_frame();
        history.record(mesh(Matrix4::from_translation(Vector3::new(1.5, 2.0, 3.0))));
        assert_eq!(pending(), 1, "it moved");
        end_frame();
    }

    #[test]
    fn a_mesh_drawn_twice_in_a_view_keeps_the_cameras_motion() {
        let history = History::default();
        begin_frame();
        history.record(mesh(Matrix4::from_scale(1.0)));
        history.record(mesh(Matrix4::from_scale(2.0)));
        begin_frame();
        history.record(mesh(Matrix4::from_scale(3.0)));
        assert_eq!(pending(), 0);
        end_frame();
    }

    #[test]
    fn a_mesh_missing_from_a_view_starts_over() {
        let history = History::default();
        begin_frame();
        history.record(mesh(Matrix4::from_scale(1.0)));
        begin_frame();
        begin_frame();
        history.record(mesh(Matrix4::from_scale(2.0)));
        assert_eq!(pending(), 0, "the last position is two views old");
        end_frame();
    }
}
