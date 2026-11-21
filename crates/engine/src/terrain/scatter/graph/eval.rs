//! Working a scatter graph out for a rectangle of ground.
//!
//! Sources lay their points on structures that span the whole world (a
//! grid for `SurfacePoints`, the chunk columns' spots for `RandomPoints`)
//! and keep those inside the rectangle, so any rectangle gets the same
//! points where it overlaps another. That lets `KeepAwayFrom` look a
//! little past the rectangle's edge for what to keep away from, and makes
//! the result the same whichever cells the ground is cut into.

use std::{
    collections::HashMap,
    f32::consts::TAU,
    rc::Rc,
};

use cgmath::{InnerSpace, Matrix4, Quaternion, Rad, Rotation3, Vector3, VectorSpace};

use crate::terrain::{chunk::scatter_spots, noise::Fractal, source::TerrainSource, CHUNK_SIZE_FLOAT};

use super::{GraphNode, NodeId, NodeKind, ScatterGraph, SplitBy};

/// What a graph asks of the ground. Every `TerrainSource` is one.
pub trait Ground {
    /// Height of the ground at world `(x, z)`.
    fn height(&self, x: f64, z: f64) -> f32;
    /// How likely something tagged `tag` grows on the ground at world
    /// `(x, y, z)`, 0..1 (see `TerrainSource::scatter_chance`).
    fn chance(&self, tag: &str, x: f64, y: f32, z: f64) -> f32;
    /// The colour models standing at world `(x, z)` take on (see
    /// `TerrainSource::scatter_tint`); white: none.
    fn tint(&self, _x: f64, _z: f64) -> [f32; 3] {
        [1.0; 3]
    }
}

impl<T: TerrainSource + ?Sized> Ground for T {
    fn height(&self, x: f64, z: f64) -> f32 {
        self.surface_height(x, z)
    }

    fn chance(&self, tag: &str, x: f64, y: f32, z: f64) -> f32 {
        self.scatter_chance(tag, x, y, z)
    }

    fn tint(&self, x: f64, z: f64) -> [f32; 3] {
        self.scatter_tint(x, z)
    }
}

/// A rectangle of ground: world `x0 <= x < x1`, `z0 <= z < z1`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x0: f32,
    pub z0: f32,
    pub x1: f32,
    pub z1: f32,
}

impl Rect {
    pub fn new(x0: f32, z0: f32, x1: f32, z1: f32) -> Self {
        Self { x0, z0, x1, z1 }
    }

    /// The square of a chunk column.
    pub fn column(cx: i32, cz: i32) -> Self {
        let (x0, z0) = (cx as f32 * CHUNK_SIZE_FLOAT, cz as f32 * CHUNK_SIZE_FLOAT);
        Self::new(x0, z0, x0 + CHUNK_SIZE_FLOAT, z0 + CHUNK_SIZE_FLOAT)
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.x0 && x < self.x1 && z >= self.z0 && z < self.z1
    }

    fn grown(&self, by: f32) -> Self {
        Self::new(self.x0 - by, self.z0 - by, self.x1 + by, self.z1 + by)
    }

    fn key(&self) -> [u32; 4] {
        [self.x0.to_bits(), self.z0.to_bits(), self.x1.to_bits(), self.z1.to_bits()]
    }
}

/// Where a model goes: on `position`, turned by `yaw` about its up axis,
/// that axis leaning to `up`, scaled by `scale` (x, up, z), its colours
/// multiplied by `tint`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub position: [f32; 3],
    pub yaw: f32,
    pub up: [f32; 3],
    pub scale: [f32; 3],
    pub tint: [f32; 3],
}

impl Placement {
    /// Standing upright on `position`, turned by `yaw`, at its own size.
    pub fn upright(position: [f32; 3], yaw: f32) -> Self {
        Self { position, yaw, up: [0.0, 1.0, 0.0], scale: [1.0; 3], tint: [1.0; 3] }
    }

    pub fn rotation(&self) -> Quaternion<f32> {
        let up = Vector3::from(self.up);
        let lean = Quaternion::from_arc(Vector3::unit_y(), up, None);
        lean * Quaternion::from_angle_y(Rad(self.yaw))
    }

    /// The model's transform. Its bottom row, always (0, 0, 0, 1) for a
    /// placement, carries the tint less one instead (see
    /// `scatter_instanced_vertex.glsl`, which puts the row back): no tint
    /// is a plain transform.
    pub fn matrix(&self) -> Matrix4<f32> {
        let mut m = Matrix4::from_translation(self.position.into())
            * Matrix4::from(self.rotation())
            * Matrix4::from_nonuniform_scale(self.scale[0], self.scale[1], self.scale[2]);
        m.x.w = self.tint[0] - 1.0;
        m.y.w = self.tint[1] - 1.0;
        m.z.w = self.tint[2] - 1.0;
        m
    }
}

/// What a graph made of a rectangle.
#[derive(Clone, Debug, Default)]
pub struct Evaluation {
    /// Per `SpawnMesh` node, in node order: its placements.
    pub outputs: Vec<(NodeId, Vec<Placement>)>,
    /// Per node: how many points it put out (for the node editor).
    pub counts: HashMap<NodeId, usize>,
    /// Per node, per output pin: where its points stand (`evaluate_traced`
    /// only; a `SpawnMesh`'s are what came in).
    pub trace: HashMap<NodeId, Vec<Vec<[f32; 3]>>>,
}

impl Evaluation {
    pub fn placements(&self, output: NodeId) -> &[Placement] {
        self.outputs
            .iter()
            .find(|(id, _)| *id == output)
            .map_or(&[], |(_, p)| p.as_slice())
    }
}

/// Most points one source may lay on one rectangle; past it the graph is
/// refused (a spacing far too fine for the rectangle).
pub const MAX_SOURCE_POINTS: usize = 2_000_000;

/// Smallest spacing of `SurfacePoints`.
const MIN_SPACING: f32 = 0.05;

/// Work `graph` out for `rect` of `ground`. Fails if the graph's structure
/// is broken (see `ScatterGraph::problems`) or a source would lay too many
/// points.
pub fn evaluate<G: Ground + ?Sized>(graph: &ScatterGraph, ground: &G, rect: Rect) -> Result<Evaluation, String> {
    run(graph, ground, rect, false)
}

/// `evaluate`, keeping every node's points too (`Evaluation::trace`), for
/// the node editor to show.
pub fn evaluate_traced<G: Ground + ?Sized>(graph: &ScatterGraph, ground: &G, rect: Rect) -> Result<Evaluation, String> {
    run(graph, ground, rect, true)
}

fn run<G: Ground + ?Sized>(graph: &ScatterGraph, ground: &G, rect: Rect, traced: bool) -> Result<Evaluation, String> {
    if let Some(problem) = graph.problems().first() {
        return Err(problem.to_string());
    }
    let mut eval = Evaluator {
        graph,
        ground,
        root: rect,
        done: HashMap::new(),
        counts: HashMap::new(),
        trace: traced.then(HashMap::new),
    };
    let mut outputs = Vec::new();
    for node in &graph.nodes {
        if let NodeKind::SpawnMesh(spawn) = &node.kind {
            let points = eval.input(node, 0, rect)?;
            eval.counts.insert(node.id, points.len());
            if let Some(trace) = &mut eval.trace {
                trace.insert(node.id, vec![points.iter().map(|p| p.position).collect()]);
            }
            let amount = spawn.tint.clamp(0.0, 1.0);
            let placements = points
                .iter()
                .map(|p| {
                    let mut placement = p.placement();
                    if amount > 0.0 {
                        let ground = ground.tint(p.position[0] as f64, p.position[2] as f64);
                        placement.tint = ground.map(|c| 1.0 + (c - 1.0) * amount);
                    }
                    placement
                })
                .collect();
            outputs.push((node.id, placements));
        }
    }
    Ok(Evaluation { outputs, counts: eval.counts, trace: eval.trace.unwrap_or_default() })
}

/// A point on its way through the graph.
#[derive(Clone, Copy, Debug)]
struct Point {
    position: [f32; 3],
    /// The ground's normal there, once a node needed it.
    normal: Option<[f32; 3]>,
    density: f32,
    /// 0..1, drawn by the source; `DensityCut` keeps the point if it is
    /// under the density.
    rand: f32,
    /// The point's own seed, for the random numbers of later nodes.
    seed: u64,
    yaw: f32,
    up: [f32; 3],
    scale: [f32; 3],
}

impl Point {
    fn new<G: Ground + ?Sized>(ground: &G, x: f32, z: f32, rand: f32, yaw: f32, seed: u64) -> Self {
        Point {
            position: [x, ground.height(x as f64, z as f64), z],
            normal: None,
            density: 1.0,
            rand,
            seed,
            yaw,
            up: [0.0, 1.0, 0.0],
            scale: [1.0; 3],
        }
    }

    fn placement(&self) -> Placement {
        Placement { position: self.position, yaw: self.yaw, up: self.up, scale: self.scale, tint: [1.0; 3] }
    }

    /// A random number 0..1 of this point for node `node`, the `k`th.
    fn rand_for(&self, node: NodeId, k: u64) -> f32 {
        unit(mix(self.seed ^ mix((node.0 as u64) << 8 | k)))
    }

    fn normal<G: Ground + ?Sized>(&mut self, ground: &G) -> Vector3<f32> {
        let n = *self.normal.get_or_insert_with(|| ground_normal(ground, self.position[0], self.position[2]));
        Vector3::from(n)
    }
}

/// The ground's normal at `(x, z)`, from its height half a metre around.
fn ground_normal<G: Ground + ?Sized>(ground: &G, x: f32, z: f32) -> [f32; 3] {
    const E: f64 = 0.5;
    let (x, z) = (x as f64, z as f64);
    let dx = ground.height(x + E, z) - ground.height(x - E, z);
    let dz = ground.height(x, z + E) - ground.height(x, z - E);
    let n = Vector3::new(-dx, 2.0 * E as f32, -dz).normalize();
    [n.x, n.y, n.z]
}

/// Points per output pin of a node.
type Outputs = Rc<Vec<Vec<Point>>>;

struct Evaluator<'a, G: Ground + ?Sized> {
    graph: &'a ScatterGraph,
    ground: &'a G,
    root: Rect,
    done: HashMap<(NodeId, [u32; 4]), Outputs>,
    counts: HashMap<NodeId, usize>,
    trace: Option<HashMap<NodeId, Vec<Vec<[f32; 3]>>>>,
}

impl<G: Ground + ?Sized> Evaluator<'_, G> {
    /// The points flowing into input `pin` of `node` on `rect`.
    fn input(&mut self, node: &GraphNode, pin: u8, rect: Rect) -> Result<Vec<Point>, String> {
        let wires: Vec<_> = self.graph.wires_into(node.id, pin).copied().collect();
        let mut points = Vec::new();
        for wire in wires {
            let from = self
                .graph
                .node(wire.from.node)
                .ok_or_else(|| format!("node {} is missing", wire.from.node.0))?;
            let outputs = self.outputs(from, rect)?;
            if let Some(out) = outputs.get(wire.from.pin as usize) {
                points.extend_from_slice(out);
            }
        }
        Ok(points)
    }

    fn outputs(&mut self, node: &GraphNode, rect: Rect) -> Result<Outputs, String> {
        let key = (node.id, rect.key());
        if let Some(done) = self.done.get(&key) {
            return Ok(done.clone());
        }
        let outputs = Rc::new(self.run(node, rect)?);
        if rect == self.root {
            self.counts.insert(node.id, outputs.iter().map(Vec::len).sum());
            if let Some(trace) = &mut self.trace {
                trace.insert(node.id, outputs.iter().map(|pin| pin.iter().map(|p| p.position).collect()).collect());
            }
        }
        self.done.insert(key, outputs.clone());
        Ok(outputs)
    }

    fn run(&mut self, node: &GraphNode, rect: Rect) -> Result<Vec<Vec<Point>>, String> {
        let ground = self.ground;
        let id = node.id;
        let one = |points: Vec<Point>| Ok(vec![points]);
        match &node.kind {
            NodeKind::SurfacePoints { spacing, jitter, seed } => one(surface_points(ground, rect, *spacing, *jitter, *seed)?),
            NodeKind::RandomPoints { per_hectare, seed } => one(random_points(ground, rect, *per_hectare, *seed)?),
            NodeKind::Slope { min, max, soft } => {
                let mut points = self.input(node, 0, rect)?;
                for p in &mut points {
                    let slope = p.normal(ground).y.clamp(-1.0, 1.0).acos().to_degrees();
                    p.density *= band(slope, *min, *max, *soft);
                }
                one(alive(points))
            }
            NodeKind::Height { min, max, soft } => {
                let mut points = self.input(node, 0, rect)?;
                for p in &mut points {
                    p.density *= band(p.position[1], *min, *max, *soft);
                }
                one(alive(points))
            }
            NodeKind::WorldChance { tag } => {
                let mut points = self.input(node, 0, rect)?;
                for p in &mut points {
                    let [x, y, z] = p.position;
                    p.density *= ground.chance(tag, x as f64, y, z as f64).clamp(0.0, 1.0);
                }
                one(alive(points))
            }
            NodeKind::Noise { wavelength, threshold, softness, seed } => {
                let mut points = self.input(node, 0, rect)?;
                let noise = Fractal::new(*seed, "scatter noise", 3, wavelength.max(0.5) as f64);
                let half = softness.max(0.0) / 2.0;
                for p in &mut points {
                    let n = (noise.at(p.position[0] as f64, p.position[2] as f64) as f32 * 0.5 + 0.5).clamp(0.0, 1.0);
                    p.density *= if half > 0.0 {
                        smoothstep(threshold - half, threshold + half, n)
                    } else {
                        (n >= *threshold) as u8 as f32
                    };
                }
                one(alive(points))
            }
            NodeKind::DensityCut => {
                let mut points = self.input(node, 0, rect)?;
                points.retain(|p| p.rand < p.density);
                one(points)
            }
            NodeKind::KeepAwayFrom { radius } => {
                let radius = radius.max(0.0);
                let mut points = self.input(node, 0, rect)?;
                let away = self.input(node, 1, rect.grown(radius))?;
                let grid = SpotGrid::new(&away, radius.max(0.01));
                points.retain(|p| !grid.any_within(p.position[0], p.position[2], radius));
                one(points)
            }
            NodeKind::Rotate { yaw_min, yaw_max, align, tilt } => {
                let mut points = self.input(node, 0, rect)?;
                for p in &mut points {
                    p.yaw = lerp(*yaw_min, *yaw_max, p.rand_for(id, 0)).to_radians();
                    let mut up = Vector3::unit_y();
                    if *align != 0.0 {
                        up = up.lerp(p.normal(ground), align.clamp(0.0, 1.0)).normalize();
                    }
                    if *tilt != 0.0 {
                        let angle = (tilt * p.rand_for(id, 1)).to_radians();
                        let turn = p.rand_for(id, 2) * TAU;
                        let side = Vector3::new(turn.cos(), 0.0, turn.sin());
                        up = (up * angle.cos() + side * angle.sin()).normalize();
                    }
                    p.up = [up.x, up.y, up.z];
                }
                one(points)
            }
            NodeKind::Scale { min, max, height_min, height_max, by_density } => {
                let mut points = self.input(node, 0, rect)?;
                for p in &mut points {
                    let s = lerp(*min, *max, p.rand_for(id, 0));
                    let h = lerp(*height_min, *height_max, p.rand_for(id, 1));
                    let fade = 1.0 - by_density.clamp(0.0, 1.0) * (1.0 - p.density.clamp(0.0, 1.0));
                    p.scale = [p.scale[0] * s * fade, p.scale[1] * s * h * fade, p.scale[2] * s * fade];
                }
                one(points)
            }
            NodeKind::Offset { min, max } => {
                let mut points = self.input(node, 0, rect)?;
                for p in &mut points {
                    p.position[1] += lerp(*min, *max, p.rand_for(id, 0));
                }
                one(points)
            }
            NodeKind::Merge => one(self.input(node, 0, rect)?),
            NodeKind::Split { by, threshold } => {
                let points = self.input(node, 0, rect)?;
                let (above, below) = points.into_iter().partition(|p| {
                    let value = match by {
                        SplitBy::Density => p.density,
                        SplitBy::Random => p.rand_for(id, 0),
                    };
                    value >= *threshold
                });
                Ok(vec![above, below])
            }
            NodeKind::SpawnMesh(_) => Ok(Vec::new()),
        }
    }
}

/// Drop the points nothing is left of.
fn alive(mut points: Vec<Point>) -> Vec<Point> {
    points.retain(|p| p.density > 0.0);
    points
}

fn surface_points<G: Ground + ?Sized>(ground: &G, rect: Rect, spacing: f32, jitter: f32, seed: u64) -> Result<Vec<Point>, String> {
    let s = spacing.max(MIN_SPACING) as f64;
    let jitter = jitter.clamp(0.0, 1.0) as f64;
    let (i0, i1) = ((rect.x0 as f64 / s).floor() as i64, (rect.x1 as f64 / s).floor() as i64);
    let (j0, j1) = ((rect.z0 as f64 / s).floor() as i64, (rect.z1 as f64 / s).floor() as i64);
    let cells = (i1 - i0 + 1).max(0) as usize * (j1 - j0 + 1).max(0) as usize;
    if cells > MAX_SOURCE_POINTS {
        return Err(format!("{cells} surface points on one piece of ground; the spacing is too fine"));
    }
    let mut points = Vec::new();
    for j in j0..=j1 {
        for i in i0..=i1 {
            let h = mix(seed ^ mix(i as u64) ^ mix((j as u64).wrapping_add(0x9E3779B97F4A7C15)));
            let r = |k: u64| unit(mix(h.wrapping_add(k)));
            let x = ((i as f64 + 0.5 + jitter * (r(1) as f64 - 0.5)) * s) as f32;
            let z = ((j as f64 + 0.5 + jitter * (r(2) as f64 - 0.5)) * s) as f32;
            if rect.contains(x, z) {
                points.push(Point::new(ground, x, z, r(3), r(4) * TAU, mix(h.wrapping_add(5))));
            }
        }
    }
    Ok(points)
}

fn random_points<G: Ground + ?Sized>(ground: &G, rect: Rect, per_hectare: f32, seed: u64) -> Result<Vec<Point>, String> {
    // As many per column as a scatter layer of that density tries.
    let attempts = (per_hectare.max(0.0) * CHUNK_SIZE_FLOAT * CHUNK_SIZE_FLOAT / (100.0 * 100.0)).round() as u32;
    let column = |v: f32| (v / CHUNK_SIZE_FLOAT).floor() as i32;
    let (cx0, cx1, cz0, cz1) = (column(rect.x0), column(rect.x1), column(rect.z0), column(rect.z1));
    let columns = (cx1 - cx0 + 1).max(0) as usize * (cz1 - cz0 + 1).max(0) as usize;
    if columns * attempts as usize > MAX_SOURCE_POINTS {
        return Err(format!("{} random points on one piece of ground; too many", columns * attempts as usize));
    }
    let mut points = Vec::new();
    for cz in cz0..=cz1 {
        for cx in cx0..=cx1 {
            for (x, z, grow, yaw) in scatter_spots((cx, cz), seed, attempts) {
                if rect.contains(x, z) {
                    let own = mix(seed ^ mix((x.to_bits() as u64) << 32 | z.to_bits() as u64));
                    points.push(Point::new(ground, x, z, grow, yaw, own));
                }
            }
        }
    }
    Ok(points)
}

/// Points sorted into squares of `cell` metres, to find the ones near a
/// spot quickly.
struct SpotGrid {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<[f32; 2]>>,
}

impl SpotGrid {
    fn new(points: &[Point], cell: f32) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<[f32; 2]>> = HashMap::new();
        for p in points {
            let (x, z) = (p.position[0], p.position[2]);
            cells.entry(((x / cell).floor() as i32, (z / cell).floor() as i32)).or_default().push([x, z]);
        }
        Self { cell, cells }
    }

    fn any_within(&self, x: f32, z: f32, radius: f32) -> bool {
        let (ci, cj) = ((x / self.cell).floor() as i32, (z / self.cell).floor() as i32);
        let reach = (radius / self.cell).ceil() as i32;
        for j in cj - reach..=cj + reach {
            for i in ci - reach..=ci + reach {
                let Some(spots) = self.cells.get(&(i, j)) else { continue };
                if spots.iter().any(|[sx, sz]| (sx - x).powi(2) + (sz - z).powi(2) <= radius * radius) {
                    return true;
                }
            }
        }
        false
    }
}

/// 1 between `min` and `max`, fading to 0 over `soft` beyond them.
fn band(value: f32, min: f32, max: f32, soft: f32) -> f32 {
    if soft <= 0.0 {
        return (value >= min && value <= max) as u8 as f32;
    }
    smoothstep(min - soft, min, value) * (1.0 - smoothstep(max, max + soft, value))
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return (x >= edge1) as u8 as f32;
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// splitmix64's finaliser.
fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

/// 0..1 from the top 24 bits.
fn unit(h: u64) -> f32 {
    (h >> 40) as f32 / (1u32 << 24) as f32
}
