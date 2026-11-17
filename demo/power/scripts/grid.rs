//! The power grid: which devices are joined by cables into a network, and
//! how the power in each network is shared out every frame.
//!
//! In each network the producers' power goes to the consumers first. What
//! is left charges the batteries (as fast as they can take it), the rest
//! is not made (a solar panel or turbine with nobody to take its power
//! runs idle). When the producers fall short the batteries make up the
//! rest; when they cannot either, every consumer gets the same share of
//! what there is (lamps dim).

use crate::devices::Role;

/// A device as the grid sees it this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Unit {
    pub role: Role,
    /// What a producer could make now, watts (sun, wind, switched on).
    pub available_w: f32,
    /// What a consumer (or a pole's lamp) wants now, watts.
    pub demand_w: f32,
    /// What a battery holds, watt hours.
    pub charge_wh: f32,
    pub capacity_wh: f32,
    /// How fast a battery charges and discharges, watts.
    pub rate_w: f32,
}

impl Unit {
    pub fn producer(available_w: f32) -> Unit {
        Unit { role: Role::Producer, available_w, ..Unit::idle(Role::Producer) }
    }

    pub fn consumer(demand_w: f32) -> Unit {
        Unit { role: Role::Consumer, demand_w, ..Unit::idle(Role::Consumer) }
    }

    pub fn battery(charge_wh: f32, capacity_wh: f32, rate_w: f32) -> Unit {
        Unit { charge_wh, capacity_wh, rate_w, ..Unit::idle(Role::Storage) }
    }

    pub fn idle(role: Role) -> Unit {
        Unit { role, available_w: 0.0, demand_w: 0.0, charge_wh: 0.0, capacity_wh: 0.0, rate_w: 0.0 }
    }
}

/// What a device did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flow {
    /// A producer: watts it made (at most what it could).
    pub produced_w: f32,
    /// A consumer: share of what it wanted that it got, 0..1.
    pub supplied: f32,
    /// A battery: watts into it (charging, > 0) or out of it (< 0).
    pub battery_w: f32,
}

/// One network's sums this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Network {
    pub devices: usize,
    /// What its producers could make.
    pub available_w: f32,
    /// What they made.
    pub produced_w: f32,
    /// What its consumers wanted.
    pub demand_w: f32,
    /// What they got.
    pub supplied_w: f32,
    /// Into its batteries (> 0) or out of them (< 0).
    pub battery_w: f32,
    pub stored_wh: f32,
    pub capacity_wh: f32,
}

impl Network {
    /// Share of the demand that is met, 0..1 (1 with no demand).
    pub fn supply(&self) -> f32 {
        if self.demand_w <= 0.0 {
            1.0
        } else {
            (self.supplied_w / self.demand_w).clamp(0.0, 1.0)
        }
    }
}

/// The network of each of `count` devices, given the cables between them
/// (`links`, by device index): numbers from 0, the same for devices joined
/// by any chain of cables, in the order the devices first appear.
pub fn networks(count: usize, links: &[(usize, usize)]) -> Vec<usize> {
    let mut parent: Vec<usize> = (0..count).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for &(a, b) in links {
        if a >= count || b >= count {
            continue;
        }
        let (ra, rb) = (root(&mut parent, a), root(&mut parent, b));
        if ra != rb {
            parent[ra.max(rb)] = ra.min(rb);
        }
    }
    let mut label = vec![usize::MAX; count];
    let mut next = 0;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let r = root(&mut parent, i);
        if label[r] == usize::MAX {
            label[r] = next;
            next += 1;
        }
        out.push(label[r]);
    }
    out
}

/// Share out the power of each network for `hours` of game time, and
/// charge or drain the batteries (their `charge_wh` changes). `network` is
/// each unit's network (see `networks`).
pub fn balance(units: &mut [Unit], network: &[usize], hours: f32) -> (Vec<Flow>, Vec<Network>) {
    let count = network.iter().copied().max().map_or(0, |m| m + 1);
    let mut nets = vec![Network::default(); count];
    let mut flows = vec![Flow::default(); units.len()];
    let hours = hours.max(1e-6);
    // What each battery can take and give in this step.
    let can_take = |u: &Unit| u.rate_w.min(((u.capacity_wh - u.charge_wh) / hours).max(0.0));
    let can_give = |u: &Unit| u.rate_w.min((u.charge_wh / hours).max(0.0));

    let mut take = vec![0.0f32; count];
    let mut give = vec![0.0f32; count];
    for (u, &n) in units.iter().zip(network) {
        let net = &mut nets[n];
        net.devices += 1;
        match u.role {
            Role::Producer => net.available_w += u.available_w.max(0.0),
            Role::Consumer | Role::Line => net.demand_w += u.demand_w.max(0.0),
            Role::Storage => {
                net.stored_wh += u.charge_wh;
                net.capacity_wh += u.capacity_wh;
                take[n] += can_take(u);
                give[n] += can_give(u);
            }
        }
    }
    // Per network: how much of what the producers could make is used, how
    // much of the demand is met, how much the batteries take or give.
    let mut produced_share = vec![0.0f32; count];
    let mut supply = vec![1.0f32; count];
    let mut charge_share = vec![0.0f32; count];
    let mut discharge_share = vec![0.0f32; count];
    for (n, net) in nets.iter_mut().enumerate() {
        if net.available_w >= net.demand_w {
            let surplus = net.available_w - net.demand_w;
            let into = surplus.min(take[n]);
            charge_share[n] = if take[n] > 0.0 { into / take[n] } else { 0.0 };
            net.battery_w = into;
            net.produced_w = net.demand_w + into;
            net.supplied_w = net.demand_w;
        } else {
            let short = net.demand_w - net.available_w;
            let out = short.min(give[n]);
            discharge_share[n] = if give[n] > 0.0 { out / give[n] } else { 0.0 };
            net.battery_w = -out;
            net.produced_w = net.available_w;
            net.supplied_w = net.available_w + out;
        }
        produced_share[n] = if net.available_w > 0.0 { net.produced_w / net.available_w } else { 0.0 };
        supply[n] = net.supply();
    }
    for ((u, &n), flow) in units.iter_mut().zip(network).zip(flows.iter_mut()) {
        match u.role {
            Role::Producer => flow.produced_w = u.available_w.max(0.0) * produced_share[n],
            Role::Consumer | Role::Line => flow.supplied = if u.demand_w > 0.0 { supply[n] } else { 0.0 },
            Role::Storage => {
                let w = can_take(u) * charge_share[n] - can_give(u) * discharge_share[n];
                flow.battery_w = w;
                u.charge_wh = (u.charge_wh + w * hours).clamp(0.0, u.capacity_wh);
            }
        }
    }
    // Stored energy after the step.
    for net in nets.iter_mut() {
        net.stored_wh = 0.0;
    }
    for (u, &n) in units.iter().zip(network) {
        if u.role == Role::Storage {
            nets[n].stored_wh += u.charge_wh;
        }
    }
    (flows, nets)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn cables_join_devices_into_networks() {
        // 0-1-2 joined, 3 alone, 4-5 joined.
        let nets = networks(6, &[(0, 1), (2, 1), (4, 5)]);
        assert_eq!(nets, vec![0, 0, 0, 1, 2, 2]);
        // Two cables between the same devices, or a loop, change nothing.
        assert_eq!(networks(3, &[(0, 1), (1, 0), (1, 2), (2, 0)]), vec![0, 0, 0]);
    }

    #[test]
    fn a_solar_panel_powers_a_lamp_and_runs_below_its_best() {
        let mut units = [Unit::producer(400.0), Unit::consumer(60.0)];
        let (flows, nets) = balance(&mut units, &[0, 0], 0.01);
        assert!(close(flows[1].supplied, 1.0));
        assert!(close(flows[0].produced_w, 60.0));
        assert!(close(nets[0].available_w, 400.0));
    }

    #[test]
    fn a_consumer_without_a_cable_gets_nothing() {
        let mut units = [Unit::producer(400.0), Unit::consumer(60.0)];
        let (flows, nets) = balance(&mut units, &networks(2, &[]), 0.01);
        assert_eq!(flows[1].supplied, 0.0);
        assert_eq!(flows[0].produced_w, 0.0);
        assert_eq!(nets.len(), 2);
    }

    #[test]
    fn too_little_power_is_shared_out_evenly() {
        // 1030 W for 2060 W (a heater and a lamp): every consumer gets half.
        let mut units = [Unit::producer(1030.0), Unit::consumer(2000.0), Unit::consumer(60.0)];
        let (flows, nets) = balance(&mut units, &[0, 0, 0], 0.01);
        assert!(close(flows[1].supplied, 0.5) && close(flows[2].supplied, 0.5));
        assert!(close(nets[0].supply(), 0.5));
        assert!(close(flows[0].produced_w, 1030.0));
    }

    #[test]
    fn surplus_charges_the_battery_up_to_its_rate() {
        let mut units = [Unit::producer(5000.0), Unit::consumer(500.0), Unit::battery(1000.0, 5000.0, 3000.0)];
        let (flows, nets) = balance(&mut units, &[0, 0, 0], 0.5);
        // 4500 W left, the battery takes 3000 W for half an hour.
        assert!(close(flows[2].battery_w, 3000.0));
        assert!(close(units[2].charge_wh, 2500.0));
        assert!(close(flows[0].produced_w, 3500.0));
        assert!(close(nets[0].stored_wh, 2500.0));
    }

    #[test]
    fn a_full_battery_takes_no_more() {
        let mut units = [Unit::producer(1000.0), Unit::battery(4990.0, 5000.0, 3000.0)];
        let (flows, _) = balance(&mut units, &[0, 0], 1.0);
        assert!(close(flows[1].battery_w, 10.0));
        assert!(close(units[1].charge_wh, 5000.0));
    }

    #[test]
    fn at_night_the_battery_keeps_the_lights_on_until_it_is_empty() {
        let mut units = [Unit::producer(0.0), Unit::consumer(100.0), Unit::battery(150.0, 5000.0, 3000.0)];
        let (flows, _) = balance(&mut units, &[0, 0, 0], 1.0);
        assert!(close(flows[1].supplied, 1.0));
        assert!(close(units[2].charge_wh, 50.0));
        // Next hour: only 50 Wh left for 100 W.
        let (flows, _) = balance(&mut units, &[0, 0, 0], 1.0);
        assert!(close(flows[1].supplied, 0.5));
        assert!(close(units[2].charge_wh, 0.0));
    }

    #[test]
    fn networks_do_not_share_power() {
        let mut units = [Unit::producer(1000.0), Unit::consumer(100.0), Unit::consumer(100.0)];
        let (flows, _) = balance(&mut units, &[0, 0, 1], 0.1);
        assert!(close(flows[1].supplied, 1.0));
        assert_eq!(flows[2].supplied, 0.0);
    }
}
