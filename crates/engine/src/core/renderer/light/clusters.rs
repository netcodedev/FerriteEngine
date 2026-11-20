//! Clustered lighting: the camera's view volume is cut into a grid of
//! clusters (tiles across the screen, slices in depth that grow with the
//! distance), and each cluster gets the list of the frame's lights that can
//! reach into it. A shaded point looks up its cluster and goes through
//! that list instead of every light (`lightsAt` in `pbr/lighting.glsl`).
//! Points outside the grid, and shaders drawn while no grid is set, go
//! through every light as before, so the light a point gets never changes.

use std::cell::RefCell;

use cgmath::{Matrix4, Point3, Transform};

use super::light::{LightKind, LightSource};
use crate::core::gfx;

/// Must match `pbr/lighting.glsl`.
pub const GRID_BINDING: u32 = 2;
pub const RANGES_BINDING: u32 = 3;
pub const INDICES_BINDING: u32 = 4;

pub const TILES_X: usize = 16;
pub const TILES_Y: usize = 9;
pub const SLICES: usize = 24;
const CLUSTERS: usize = TILES_X * TILES_Y * SLICES;

/// How far past its range a light's bounce reaches (`BOUNCE_REACH` in
/// `pbr/lighting.glsl`, plus the 0.3 ranges a spot light's bounce source
/// sits ahead of it).
const BOUNCE_EXTENT: f32 = 2.0 + 0.3;

/// The camera a grid is built for.
#[derive(Clone, Copy, Debug)]
pub struct ClusterCamera {
    /// World to view space (the camera looks down -z).
    pub view: Matrix4<f32>,
    pub projection: Matrix4<f32>,
    pub near: f32,
    pub far: f32,
}

/// Each cluster's lights, as the shaders read them: per cluster the first
/// index into `indices` and how many, and the light numbers.
#[derive(Default, Debug)]
pub struct ClusterLists {
    pub ranges: Vec<[u32; 2]>,
    pub indices: Vec<u32>,
}

/// The cluster of tile `(x, y)` and slice `z`.
pub fn cluster_index(x: usize, y: usize, z: usize) -> usize {
    (z * TILES_Y + y) * TILES_X + x
}

impl ClusterCamera {
    /// The slice at `depth` metres in front of the camera.
    fn slice(&self, depth: f32) -> usize {
        let depth = depth.clamp(self.near, self.far);
        let t = (depth / self.near).ln() / (self.far / self.near).ln();
        ((t * SLICES as f32) as usize).min(SLICES - 1)
    }

    /// The clusters a sphere around view-space `center` may reach into:
    /// tile ranges and slice range, inclusive. `None` if none.
    fn reach(&self, center: Point3<f32>, radius: f32) -> Option<([usize; 2], [usize; 2], [usize; 2])> {
        let (near_depth, far_depth) = (-center.z - radius, -center.z + radius);
        if far_depth < self.near || near_depth > self.far {
            return None;
        }
        let slices = [self.slice(near_depth), self.slice(far_depth)];
        let full = ([0, TILES_X - 1], [0, TILES_Y - 1], slices);
        if near_depth <= self.near {
            // Reaches to the camera: it may cover any part of the screen.
            return Some(full);
        }
        // The corners of the box around the sphere, projected: the box is
        // in front of the camera, so its picture lies within theirs.
        let (sx, sy) = (self.projection.x.x, self.projection.y.y);
        let mut ndc_x = [f32::MAX, f32::MIN];
        let mut ndc_y = [f32::MAX, f32::MIN];
        for depth in [near_depth, far_depth] {
            for x in [center.x - radius, center.x + radius] {
                let v = sx * x / depth;
                ndc_x = [ndc_x[0].min(v), ndc_x[1].max(v)];
            }
            for y in [center.y - radius, center.y + radius] {
                let v = sy * y / depth;
                ndc_y = [ndc_y[0].min(v), ndc_y[1].max(v)];
            }
        }
        if ndc_x[0] > 1.0 || ndc_x[1] < -1.0 || ndc_y[0] > 1.0 || ndc_y[1] < -1.0 {
            return None;
        }
        let tile = |ndc: f32, tiles: usize| {
            (((ndc.clamp(-1.0, 1.0) * 0.5 + 0.5) * tiles as f32) as usize).min(tiles - 1)
        };
        Some((
            [tile(ndc_x[0], TILES_X), tile(ndc_x[1], TILES_X)],
            [tile(ndc_y[0], TILES_Y), tile(ndc_y[1], TILES_Y)],
            slices,
        ))
    }

    /// Which of `lights` (by their number in the slice) each cluster gets.
    pub fn assign(&self, lights: &[LightSource]) -> ClusterLists {
        let reaches: Vec<Option<_>> = lights
            .iter()
            .map(|light| match light.kind {
                LightKind::Directional => {
                    Some(([0, TILES_X - 1], [0, TILES_Y - 1], [0, SLICES - 1]))
                }
                _ => {
                    let extent = if light.bounce > 0.0 { BOUNCE_EXTENT.max(1.0) } else { 1.0 };
                    self.reach(self.view.transform_point(light.position), light.range * extent)
                }
            })
            .collect();
        let mut counts = vec![0u32; CLUSTERS];
        let each = |f: &mut dyn FnMut(usize, usize)| {
            for (number, reach) in reaches.iter().enumerate() {
                let Some((xs, ys, zs)) = reach else { continue };
                for z in zs[0]..=zs[1] {
                    for y in ys[0]..=ys[1] {
                        for x in xs[0]..=xs[1] {
                            f(cluster_index(x, y, z), number);
                        }
                    }
                }
            }
        };
        each(&mut |cluster, _| counts[cluster] += 1);
        let mut ranges = Vec::with_capacity(CLUSTERS);
        let mut first = 0u32;
        for &count in &counts {
            ranges.push([first, 0]);
            first += count;
        }
        let mut indices = vec![0u32; first as usize];
        each(&mut |cluster, number| {
            let [start, filled] = &mut ranges[cluster];
            indices[(*start + *filled) as usize] = number as u32;
            *filled += 1;
        });
        ClusterLists { ranges, indices }
    }
}

/// The grid's buffers: its camera (a uniform block) and the lists (two
/// storage buffers).
pub struct ClusterBuffers {
    grid: RefCell<gfx::Buffer>,
    ranges: gfx::Buffer,
    indices: gfx::Buffer,
}

/// std140 layout of `LightClusterGrid`.
#[repr(C)]
#[derive(Clone, Copy)]
struct GridBlock {
    view: [[f32; 4]; 4],
    /// x, y: the projection's scale, z: near, w: ln(far / near).
    projection: [f32; 4],
    /// Tiles across, tiles up, slices, and 1 while the grid is in use.
    dims: [i32; 4],
}

impl ClusterBuffers {
    pub fn new() -> Self {
        let storage = || gfx::Buffer::new(gfx::BufferUsage::STORAGE, gfx::BufferMemory::Dynamic, 4);
        let buffers = Self {
            grid: RefCell::new(gfx::Buffer::new(
                gfx::BufferUsage::UNIFORM,
                gfx::BufferMemory::Dynamic,
                std::mem::size_of::<GridBlock>(),
            )),
            ranges: storage(),
            indices: storage(),
        };
        buffers.write_grid(None);
        buffers
    }

    fn write_grid(&self, camera: Option<&ClusterCamera>) {
        let block = match camera {
            Some(camera) => GridBlock {
                view: camera.view.into(),
                projection: [
                    camera.projection.x.x,
                    camera.projection.y.y,
                    camera.near,
                    (camera.far / camera.near).ln(),
                ],
                dims: [TILES_X as i32, TILES_Y as i32, SLICES as i32, 1],
            },
            None => GridBlock {
                view: [[0.0; 4]; 4],
                projection: [0.0; 4],
                dims: [0; 4],
            },
        };
        let mut grid = self.grid.borrow_mut();
        grid.replace(gfx::as_bytes(std::slice::from_ref(&block)));
        gfx::bind_uniform_buffer(GRID_BINDING, &grid);
    }

    /// Build the grid for `camera` over `lights` (numbered as uploaded)
    /// and bind it for the shaders.
    pub fn upload(&mut self, camera: &ClusterCamera, lights: &[LightSource]) {
        let lists = camera.assign(lights);
        let storage = |buffer: &mut gfx::Buffer, binding: u32, bytes: &[u8]| {
            // Never empty: a zero-sized storage buffer cannot be bound.
            let data = if bytes.is_empty() { &[0u8; 4][..] } else { bytes };
            buffer.replace(data);
            // Exactly the data: the shaders take the lists' lengths from
            // the bound range.
            gfx::bind_storage_range(binding, buffer, 0, data.len());
        };
        storage(&mut self.ranges, RANGES_BINDING, gfx::as_bytes(&lists.ranges));
        storage(&mut self.indices, INDICES_BINDING, gfx::as_bytes(&lists.indices));
        self.write_grid(Some(camera));
    }

    /// Stop using the grid: shaders go through every light.
    pub fn disable(&self) {
        self.write_grid(None);
    }
}

#[cfg(test)]
mod tests {
    use cgmath::{perspective, Deg, InnerSpace, Vector3, Vector4};

    use super::*;

    fn camera() -> ClusterCamera {
        ClusterCamera {
            view: Matrix4::look_at_rh(
                Point3::new(0.0, 2.0, 0.0),
                Point3::new(0.0, 2.0, -10.0),
                Vector3::unit_y(),
            ),
            projection: perspective(Deg(70.0), 16.0 / 9.0, 0.1, 500.0),
            near: 0.1,
            far: 500.0,
        }
    }

    fn point_light(x: f32, y: f32, z: f32, range: f32) -> LightSource {
        LightSource {
            kind: LightKind::Point,
            position: Point3::new(x, y, z),
            direction: Vector3::new(0.0, -1.0, 0.0),
            radiance: [1.0; 3],
            range,
            cos_inner: 1.0,
            cos_outer: 1.0,
            bounce: 0.0,
            casts_shadows: false,
            shadow_layer: None,
        }
    }

    /// The cluster of world point `p`, as the shader finds it.
    fn cluster_of(camera: &ClusterCamera, p: Point3<f32>) -> Option<usize> {
        let v = camera.view.transform_point(p);
        let depth = -v.z;
        if depth <= 0.0 || depth > camera.far {
            return None;
        }
        let clip = camera.projection * Vector4::new(v.x, v.y, v.z, 1.0);
        let (x, y) = (clip.x / clip.w, clip.y / clip.w);
        if x.abs() > 1.0 || y.abs() > 1.0 {
            return None;
        }
        let tile = |ndc: f32, n: usize| (((ndc * 0.5 + 0.5) * n as f32) as usize).min(n - 1);
        Some(cluster_index(tile(x, TILES_X), tile(y, TILES_Y), camera.slice(depth)))
    }

    #[test]
    fn every_point_a_light_reaches_has_it_in_its_cluster() {
        let camera = camera();
        let lights = vec![
            point_light(3.0, 1.0, -12.0, 5.0),
            point_light(-20.0, 4.0, -60.0, 12.0),
            point_light(0.0, 2.0, 1.0, 3.0), // around the camera
            point_light(40.0, 0.0, 30.0, 4.0), // behind it
        ];
        let lists = camera.assign(&lights);
        let mut seed = 7u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        let mut checked = 0;
        for (number, light) in lights.iter().enumerate() {
            for _ in 0..4000 {
                let dir = Vector3::new(random() - 0.5, random() - 0.5, random() - 0.5);
                if dir.magnitude() < 1e-3 {
                    continue;
                }
                let p = light.position + dir.normalize() * light.range * random();
                let Some(cluster) = cluster_of(&camera, p) else { continue };
                let [first, count] = lists.ranges[cluster];
                let list = &lists.indices[first as usize..(first + count) as usize];
                assert!(list.contains(&(number as u32)), "light {number} missing at {p:?}");
                checked += 1;
            }
        }
        assert!(checked > 1000);
    }

    #[test]
    fn far_off_lights_stay_out_of_most_clusters() {
        let camera = camera();
        let lists = camera.assign(&[point_light(3.0, 1.0, -40.0, 2.0)]);
        let holding = lists.ranges.iter().filter(|r| r[1] > 0).count();
        assert!(holding > 0 && holding < CLUSTERS / 50, "{holding}");
        // Behind the camera: nowhere.
        let lists = camera.assign(&[point_light(0.0, 2.0, 40.0, 2.0)]);
        assert!(lists.indices.is_empty());
    }

    #[test]
    fn directional_lights_are_everywhere() {
        let mut sun = point_light(0.0, 0.0, 0.0, 1.0);
        sun.kind = LightKind::Directional;
        let lists = camera().assign(&[sun]);
        assert!(lists.ranges.iter().all(|r| r[1] == 1));
    }
}
