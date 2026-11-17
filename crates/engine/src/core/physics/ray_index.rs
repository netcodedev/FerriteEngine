//! Where the colliders are, for ray casts and overlap tests that must work
//! while the simulation is paused (picking and placing in the editor), so
//! they cannot use rapier's broad phase, which only a physics step brings
//! up to date.
//!
//! The colliders of fixed bodies (terrain, trees, buildings) are sorted
//! into a grid of `CELL` metre squares across the ground (x, z), each
//! listed in every square its box covers; a query looks only at the
//! squares it passes. The others are few and move (players, things lying
//! around): they are looked at by every query, as are colliders without a
//! body and very wide ones.
//!
//! Bodies and colliders change behind the physics engine's back (a door is
//! swung by setting its fixed body's pose, a scene moves an entity's
//! body), so `sync` compares what the grid was made from with what there
//! is: before every query how many colliders and bodies there are, and
//! once a frame (see `begin_frame`, or right away after `body_moved`)
//! where each fixed body stands. Only what changed is sorted in again.

use std::collections::{HashMap, HashSet};

use rapier3d::prelude::*;

/// Edge of a grid square, metres.
const CELL: f32 = 32.0;
/// A collider covering more squares than this goes with the moving ones.
const MAX_CELLS: i64 = 256;

type Key = (i32, i32);

/// Where an indexed collider is listed.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Placed {
    /// In the squares `min..=max`.
    Cells { min: Key, max: Key },
    /// With the colliders every query looks at.
    Loose,
}

#[derive(Default)]
pub(super) struct RayIndex {
    cells: HashMap<Key, Vec<ColliderHandle>>,
    loose: Vec<ColliderHandle>,
    placed: HashMap<ColliderHandle, Placed>,
    /// Every body's pose and whether it was fixed, when its colliders were
    /// sorted in, by the body handle's index (with its generation).
    bodies: Vec<Option<(u32, Pose, bool)>>,
    /// How many colliders and bodies there were then.
    colliders_seen: usize,
    bodies_seen: usize,
    /// Set by the physics engine whenever it adds or removes something, in
    /// case as many were added as removed.
    pub(super) dirty: bool,
    /// Whether the fixed bodies' poses are to be looked at again (a new
    /// frame began, or one was moved).
    pub(super) poses_stale: bool,
}

fn key(x: f32, z: f32) -> Key {
    ((x / CELL).floor() as i32, (z / CELL).floor() as i32)
}

impl RayIndex {
    /// Bring the grid in line with `bodies` and `colliders` (whose poses
    /// are propagated already).
    pub(super) fn sync(&mut self, bodies: &RigidBodySet, colliders: &ColliderSet) {
        let counts_changed = colliders.len() != self.colliders_seen || bodies.len() != self.bodies_seen;
        if self.dirty || counts_changed {
            // Gone ones out, new ones in.
            let gone: Vec<ColliderHandle> =
                self.placed.keys().copied().filter(|&h| !colliders.contains(h)).collect();
            for handle in gone {
                self.take_out(handle);
            }
            for (handle, collider) in colliders.iter() {
                if !self.placed.contains_key(&handle) {
                    self.put_in(handle, collider, bodies);
                }
            }
            for (index, slot) in self.bodies.iter_mut().enumerate() {
                if slot.is_some_and(|(generation, _, _)| {
                    !bodies.contains(RigidBodyHandle::from_raw_parts(index as u32, generation))
                }) {
                    *slot = None;
                }
            }
            self.colliders_seen = colliders.len();
            self.bodies_seen = bodies.len();
            self.dirty = false;
        }
        if !std::mem::take(&mut self.poses_stale) {
            return;
        }
        // Bodies that moved (or became fixed or stopped being): their
        // colliders sorted in again.
        let mut moved = Vec::new();
        for (handle, body) in bodies.iter() {
            let (index, generation) = handle.into_raw_parts();
            let fixed = body.is_fixed();
            match self.bodies.get(index as usize).copied().flatten() {
                Some((g, pose, f)) if g == generation && f == fixed && (!fixed || pose == *body.position()) => {}
                _ => moved.push(handle),
            }
        }
        for handle in moved {
            self.remember(handle, &bodies[handle]);
            for &collider in bodies[handle].colliders() {
                if let Some(c) = colliders.get(collider) {
                    self.take_out(collider);
                    self.put_in(collider, c, bodies);
                }
            }
        }
    }

    fn put_in(&mut self, handle: ColliderHandle, collider: &Collider, bodies: &RigidBodySet) {
        let fixed = collider.parent().and_then(|p| bodies.get(p)).is_some_and(|b| b.is_fixed());
        let placed = if fixed {
            let aabb = collider.compute_aabb();
            let (min, max) = (key(aabb.mins.x, aabb.mins.z), key(aabb.maxs.x, aabb.maxs.z));
            let count = (max.0 as i64 - min.0 as i64 + 1) * (max.1 as i64 - min.1 as i64 + 1);
            if aabb.mins.x.is_finite() && aabb.maxs.z.is_finite() && count <= MAX_CELLS {
                Placed::Cells { min, max }
            } else {
                Placed::Loose
            }
        } else {
            Placed::Loose
        };
        match placed {
            Placed::Cells { min, max } => {
                for x in min.0..=max.0 {
                    for z in min.1..=max.1 {
                        self.cells.entry((x, z)).or_default().push(handle);
                    }
                }
            }
            Placed::Loose => self.loose.push(handle),
        }
        if let Some(parent) = collider.parent() {
            if let Some(body) = bodies.get(parent) {
                self.remember(parent, body);
            }
        }
        self.placed.insert(handle, placed);
    }

    /// Note where `body` stands and whether it is fixed.
    fn remember(&mut self, handle: RigidBodyHandle, body: &RigidBody) {
        let (index, generation) = handle.into_raw_parts();
        let index = index as usize;
        if self.bodies.len() <= index {
            self.bodies.resize(index + 1, None);
        }
        self.bodies[index] = Some((generation, *body.position(), body.is_fixed()));
    }

    fn take_out(&mut self, handle: ColliderHandle) {
        match self.placed.remove(&handle) {
            Some(Placed::Cells { min, max }) => {
                for x in min.0..=max.0 {
                    for z in min.1..=max.1 {
                        if let Some(list) = self.cells.get_mut(&(x, z)) {
                            if let Some(i) = list.iter().position(|&h| h == handle) {
                                list.swap_remove(i);
                            }
                            if list.is_empty() {
                                self.cells.remove(&(x, z));
                            }
                        }
                    }
                }
            }
            Some(Placed::Loose) => {
                if let Some(i) = self.loose.iter().position(|&h| h == handle) {
                    self.loose.swap_remove(i);
                }
            }
            None => {}
        }
    }

    /// Visit the colliders `ray` may hit within `max_distance`, nearest
    /// squares first: the loose ones, then each square's (each collider
    /// once). `visit` gets a collider and returns how far along the ray the
    /// nearest hit so far is; once that lies within the squares passed,
    /// the rest are left out (a hit lies in its collider's box, which is
    /// listed in the square the hit is in).
    pub(super) fn on_ray(&self, ray: &Ray, max_distance: f32, mut visit: impl FnMut(ColliderHandle) -> f32) {
        let mut best = f32::MAX;
        for &handle in &self.loose {
            best = best.min(visit(handle));
        }
        let mut seen = HashSet::new();
        let mut look = |cell: Key, best: &mut f32| {
            if let Some(list) = self.cells.get(&cell) {
                for &handle in list {
                    if seen.insert(handle) {
                        *best = best.min(visit(handle));
                    }
                }
            }
        };
        // Through the squares along the ray across the ground (Amanatides
        // and Woo), each with the distance at which the ray leaves it.
        let (ox, oz) = (ray.origin.x, ray.origin.z);
        let (dx, dz) = (ray.dir.x, ray.dir.z);
        let mut cell = key(ox, oz);
        let step = |d: f32| if d > 0.0 { 1 } else { -1 };
        let next = |o: f32, d: f32, c: i32| {
            if d.abs() < 1e-12 {
                return (f32::INFINITY, f32::INFINITY);
            }
            let edge = if d > 0.0 { (c + 1) as f32 * CELL } else { c as f32 * CELL };
            ((edge - o) / d, CELL / d.abs())
        };
        let (mut tx, ddx) = next(ox, dx, cell.0);
        let (mut tz, ddz) = next(oz, dz, cell.1);
        // (However long the ray: it stops after this many squares.)
        for _ in 0..100_000 {
            look(cell, &mut best);
            let leave = tx.min(tz);
            if best <= leave || leave > max_distance {
                break;
            }
            if tx < tz {
                cell.0 += step(dx);
                tx += ddx;
            } else {
                cell.1 += step(dz);
                tz += ddz;
            }
        }
    }

    /// Every collider whose box may overlap `aabb`.
    pub(super) fn in_box(&self, aabb: &Aabb) -> Vec<ColliderHandle> {
        let (min, max) = (key(aabb.mins.x, aabb.mins.z), key(aabb.maxs.x, aabb.maxs.z));
        let mut found: Vec<ColliderHandle> = self.loose.clone();
        let mut seen = HashSet::new();
        if (max.0 as i64 - min.0 as i64 + 1) * (max.1 as i64 - min.1 as i64 + 1) > 4 * MAX_CELLS {
            // Wider than any square list is worth: every indexed one.
            found.extend(self.placed.iter().filter(|(_, p)| **p != Placed::Loose).map(|(h, _)| *h));
            return found;
        }
        for x in min.0..=max.0 {
            for z in min.1..=max.1 {
                for &handle in self.cells.get(&(x, z)).into_iter().flatten() {
                    if seen.insert(handle) {
                        found.push(handle);
                    }
                }
            }
        }
        found
    }
}
