//! Workload families: each maps explicit parameters to a P-graph structure and,
//! **independently**, to an [`ExpectedStructure`] oracle computed by formula.
//!
//! `seed` appears only on the stochastic families (Random*, ServiceDependency,
//! ChickenFlockContact); the deterministic families carry none, so the type
//! cannot claim a seed changes a model it does not.

use super::rng::SplitMix64;
use super::{
    Built, ExpectedStructure, Structure, WorkloadError, assemble, cadd, cmul, default_names,
    default_unit_names,
};

/// A workload family with its explicit parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkloadFamily {
    // ── mathematical (kept, byte-identical goldens) ──
    /// Fano plane STS(2,3,7), fixed labelling (kept for the original golden).
    Fano,
    /// A sunflower: `petals` blocks sharing a `core`, each with `petal` private.
    Sunflower {
        core: usize,
        petal: usize,
        petals: usize,
    },
    /// The 5-cycle C5.
    MycielskiC5,
    /// The Grötzsch graph (Mycielskian of C5).
    MycielskiGrotzsch,
    /// Spine/leaf network (one unit per spine–leaf link).
    SpineLeaf { spines: usize, leaves: usize },
    /// A serial robot chain of `joints` links (primitive control).
    RobotChain { joints: usize },

    // ── mathematical (parametric) ──
    /// Steiner triple system from PG(m−1,2): v = 2^m − 1 points, blocks
    /// {a, b, a⊕b}. m∈{3→STS7, 4→STS15, 5→STS31, …}.
    Steiner { m: u32 },
    /// Mycielskian iterated `iterations` times from C5: n'=2n+1, m'=3m+n.
    Mycielski { iterations: u32 },

    // ── random (seeded) ──
    /// Each edge has `card` distinct uniform-random vertices.
    RandomUniform {
        nodes: usize,
        edges: usize,
        card: usize,
        seed: u64,
    },
    /// Controlled edge cardinality (cycled through `[min_card, max_card]`),
    /// random distinct vertices.
    RandomDegree {
        nodes: usize,
        edges: usize,
        min_card: usize,
        max_card: usize,
        seed: u64,
    },
    /// Hub-skewed: each of `card` distinct slots is a hub (first `hubs`
    /// vertices) with probability `hub_ratio_num/hub_ratio_den`, else a
    /// non-hub.
    RandomHub {
        nodes: usize,
        edges: usize,
        hubs: usize,
        hub_ratio_num: u64,
        hub_ratio_den: u64,
        card: usize,
        seed: u64,
    },

    // ── synthetic networks ──
    /// VLAN / failure-domain: hosts grouped into VLAN hyperedges, switches into
    /// power/failure-domain hyperedges.
    Vlan {
        switches: usize,
        hosts_per_switch: usize,
        vlans: usize,
        power_domains: usize,
    },
    /// Service dependency: each service instance depends on `dep_card` shared
    /// resources (databases/caches/buses).
    ServiceDependency {
        services: usize,
        replicas: usize,
        shared_groups: usize,
        dep_card: usize,
        seed: u64,
    },

    // ── robotics ──
    /// Humanoid: torso/head, two arms, two legs; per joint sensors, actuators,
    /// a local controller; shared power and communication buses.
    Humanoid {
        arm_joints: usize,
        leg_joints: usize,
        sensors_per_joint: usize,
        actuator_redundancy: usize,
    },
    /// Hexapod: `legs` legs (default 6), each with `joints_per_leg` joints.
    Hexapod {
        legs: usize,
        joints_per_leg: usize,
        sensors_per_joint: usize,
        actuator_redundancy: usize,
    },

    // ── synthetic biomedical (structural benchmarks, NOT validated models) ──
    /// A fixed chicken-physiology system: organs/tissues related by digestive,
    /// respiratory, circulatory, metabolic and musculoskeletal process
    /// hyperedges.
    ChickenPhysiology,
    /// A flock-contact model: birds sharing coops, feeders, drinkers and
    /// time-window contact groups (multiway hyperedges).
    ChickenFlockContact {
        birds: usize,
        coops: usize,
        feeders: usize,
        drinkers: usize,
        group: usize,
        windows: usize,
        seed: u64,
    },
}

/// Build a workload: its HyMeKo source and its formula oracle.
///
/// # Errors
/// [`WorkloadError`] for a degenerate parameter, a size overflow, or an
/// over-budget source.
pub fn build(family: WorkloadFamily) -> Result<Built, WorkloadError> {
    use WorkloadFamily as F;
    match family {
        F::Fano => fano(),
        F::Sunflower {
            core,
            petal,
            petals,
        } => sunflower(core, petal, petals),
        F::MycielskiC5 => mycielski_fixed(0, "mycielski_c5"),
        F::MycielskiGrotzsch => mycielski_fixed(1, "mycielski_grotzsch"),
        F::SpineLeaf { spines, leaves } => spine_leaf(spines, leaves),
        F::RobotChain { joints } => robot_chain(joints),
        F::Steiner { m } => steiner(m),
        F::Mycielski { iterations } => mycielski(iterations),
        F::RandomUniform {
            nodes,
            edges,
            card,
            seed,
        } => random_uniform(nodes, edges, card, seed),
        F::RandomDegree {
            nodes,
            edges,
            min_card,
            max_card,
            seed,
        } => random_degree(nodes, edges, min_card, max_card, seed),
        F::RandomHub {
            nodes,
            edges,
            hubs,
            hub_ratio_num,
            hub_ratio_den,
            card,
            seed,
        } => random_hub(nodes, edges, hubs, hub_ratio_num, hub_ratio_den, card, seed),
        F::Vlan {
            switches,
            hosts_per_switch,
            vlans,
            power_domains,
        } => vlan(switches, hosts_per_switch, vlans, power_domains),
        F::ServiceDependency {
            services,
            replicas,
            shared_groups,
            dep_card,
            seed,
        } => service_dependency(services, replicas, shared_groups, dep_card, seed),
        F::Humanoid {
            arm_joints,
            leg_joints,
            sensors_per_joint,
            actuator_redundancy,
        } => humanoid(
            arm_joints,
            leg_joints,
            sensors_per_joint,
            actuator_redundancy,
        ),
        F::Hexapod {
            legs,
            joints_per_leg,
            sensors_per_joint,
            actuator_redundancy,
        } => hexapod(legs, joints_per_leg, sensors_per_joint, actuator_redundancy),
        F::ChickenPhysiology => chicken_physiology(),
        F::ChickenFlockContact {
            birds,
            coops,
            feeders,
            drinkers,
            group,
            windows,
            seed,
        } => chicken_flock(birds, coops, feeders, drinkers, group, windows, seed),
    }
}

/// The oracle for a structure whose vertex count and edge sizes are known: one
/// raw (v0), one product (v_last), one arc per unit.
fn oracle_of(vertices: usize, incidences: usize, units: usize) -> ExpectedStructure {
    ExpectedStructure {
        materials: vertices,
        units,
        arcs: units,
        incidences,
        raws: 1,
        products: 1,
    }
}

/// Finish a deterministic family from an explicit vertex count, edge list and
/// (independently-computed) oracle, with default `m{i}`/`u{j}` names.
fn finish_generic(
    root: &str,
    vertices: usize,
    edges: Vec<Vec<usize>>,
    oracle: ExpectedStructure,
) -> Result<Built, WorkloadError> {
    let names = default_names(vertices);
    let unit_names = default_unit_names(edges.len());
    assemble(
        root,
        Structure {
            vertices,
            edges,
            names,
            unit_names,
        },
        oracle,
    )
}

/// Finish a domain family with explicit semantic names.
fn finish_named(
    root: &str,
    edges: Vec<Vec<usize>>,
    names: Vec<String>,
    unit_names: Vec<String>,
    oracle: ExpectedStructure,
) -> Result<Built, WorkloadError> {
    let vertices = names.len();
    assemble(
        root,
        Structure {
            vertices,
            edges,
            names,
            unit_names,
        },
        oracle,
    )
}

// ── mathematical (kept) ────────────────────────────────────────────────────

fn fano() -> Result<Built, WorkloadError> {
    let edges = vec![
        vec![0, 1, 2],
        vec![0, 3, 4],
        vec![0, 5, 6],
        vec![1, 3, 5],
        vec![1, 4, 6],
        vec![2, 3, 6],
        vec![2, 4, 5],
    ];
    let oracle = oracle_of(7, 21, 7);
    finish_generic("steiner_fano_7", 7, edges, oracle)
}

fn sunflower(core: usize, petal: usize, petals: usize) -> Result<Built, WorkloadError> {
    if petals == 0 || (core == 0 && petal == 0) {
        return Err(WorkloadError::InvalidParameter("empty sunflower"));
    }
    // Formula oracle first (checked, before allocation).
    let n = cadd(
        core,
        cmul(petals, petal, "petals*petal")?,
        "core+petals*petal",
    )?;
    let card = cadd(core, petal, "core+petal")?;
    let incidences = cmul(petals, card, "petals*card")?;
    let oracle = oracle_of(n, incidences, petals);

    let mut edges = Vec::with_capacity(petals);
    for p in 0..petals {
        let mut e: Vec<usize> = (0..core).collect();
        let base = core + p * petal;
        e.extend(base..base + petal);
        edges.push(e);
    }
    finish_generic("sunflower", n, edges, oracle)
}

fn spine_leaf(spines: usize, leaves: usize) -> Result<Built, WorkloadError> {
    if spines == 0 || leaves == 0 {
        return Err(WorkloadError::InvalidParameter("empty spine/leaf"));
    }
    let n = cadd(spines, leaves, "spines+leaves")?;
    let units = cmul(spines, leaves, "spines*leaves")?;
    let incidences = cmul(units, 2, "units*2")?;
    let oracle = oracle_of(n, incidences, units);

    let mut edges = Vec::with_capacity(units);
    for s in 0..spines {
        for l in 0..leaves {
            edges.push(vec![s, spines + l]);
        }
    }
    finish_generic("network_spine_leaf", n, edges, oracle)
}

fn robot_chain(joints: usize) -> Result<Built, WorkloadError> {
    if joints == 0 {
        return Err(WorkloadError::InvalidParameter("empty chain"));
    }
    let n = cadd(joints, 1, "joints+1")?;
    let incidences = cmul(joints, 2, "joints*2")?;
    let oracle = oracle_of(n, incidences, joints);
    let edges = (0..joints).map(|j| vec![j, j + 1]).collect();
    finish_generic("robot_chain", n, edges, oracle)
}

// ── Mycielski (fixed + parametric) ─────────────────────────────────────────

/// Iterate the Mycielskian starting from C5. Returns `(vertices, edges)`.
fn mycielski_edges(iterations: u32) -> (usize, Vec<Vec<usize>>) {
    // C5.
    let mut n = 5usize;
    let mut edges: Vec<(usize, usize)> = vec![(0, 1), (1, 2), (2, 3), (3, 4), (4, 0)];
    for _ in 0..iterations {
        // Mycielskian: originals 0..n, shadows n..2n (shadow i = n+i), apex 2n.
        let apex = 2 * n;
        let mut next: Vec<(usize, usize)> = Vec::new();
        // keep original edges
        for &(a, b) in &edges {
            next.push((a, b));
            // shadow(a)~b and shadow(b)~a
            next.push((n + a, b));
            next.push((n + b, a));
        }
        // each shadow ~ apex
        for i in 0..n {
            next.push((n + i, apex));
        }
        edges = next;
        n = 2 * n + 1;
    }
    (n, edges.into_iter().map(|(a, b)| vec![a, b]).collect())
}

fn mycielski_fixed(iterations: u32, root: &str) -> Result<Built, WorkloadError> {
    let (n, edges) = mycielski_edges(iterations);
    let m = edges.len();
    let incidences = cmul(m, 2, "edges*2")?;
    let oracle = oracle_of(n, incidences, m);
    finish_generic(root, n, edges, oracle)
}

fn mycielski(iterations: u32) -> Result<Built, WorkloadError> {
    // Formula oracle: n_{k+1}=2n_k+1, m_{k+1}=3m_k+n_k, from (n0,m0)=(5,5).
    let mut n: usize = 5;
    let mut m: usize = 5;
    for _ in 0..iterations {
        m = cadd(cmul(3, m, "3m")?, n, "3m+n")?;
        n = cadd(cmul(2, n, "2n")?, 1, "2n+1")?;
    }
    let incidences = cmul(m, 2, "edges*2")?;
    let oracle = oracle_of(n, incidences, m);
    let (bn, edges) = mycielski_edges(iterations);
    // cross-check construction vs formula happens in assemble; also verify n.
    assert_eq!(bn, n, "mycielski vertex formula vs construction");
    finish_generic(&format!("mycielski_iter{iterations}"), n, edges, oracle)
}

// ── Steiner (parametric, PG(m-1,2)) ────────────────────────────────────────

fn steiner(m: u32) -> Result<Built, WorkloadError> {
    if !(3..=13).contains(&m) {
        return Err(WorkloadError::InvalidParameter(
            "steiner rank m must be in 3..=13",
        ));
    }
    // v = 2^m - 1 (checked via shift).
    let v = (1usize
        .checked_shl(m)
        .ok_or(WorkloadError::ArithmeticOverflow("2^m"))?)
    .checked_sub(1)
    .ok_or(WorkloadError::ArithmeticOverflow("2^m-1"))?;
    // b = v(v-1)/6.
    let b = cmul(v, v - 1, "v*(v-1)")? / 6;
    let incidences = cmul(3, b, "3b")?;
    let oracle = oracle_of(v, incidences, b);
    // Guard before building the O(v^2) block loop.
    super::guard_size(&oracle)?;

    // Points 1..=v are the nonzero GF(2)^m vectors; block {a,b,a^b} with
    // a<b<a^b canonical. 0-index by subtracting 1.
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(b);
    for a in 1..=v {
        for bb in (a + 1)..=v {
            let c = a ^ bb;
            if c > bb {
                edges.push(vec![a - 1, bb - 1, c - 1]);
            }
        }
    }
    let root = format!("steiner_sts{v}");
    finish_generic(&root, v, edges, oracle)
}

// ── random (seeded) ────────────────────────────────────────────────────────

/// Sample `card` distinct vertices from `0..nodes` by partial Fisher–Yates over
/// a lazily-materialised index, deterministic in the RNG.
fn distinct_sample(rng: &mut SplitMix64, nodes: usize, card: usize) -> Vec<usize> {
    // For small card relative to nodes, rejection sampling is simplest and stays
    // O(card) expected; we cap attempts and fall back to a scan to stay total.
    let mut chosen = Vec::with_capacity(card);
    let mut guard = 0usize;
    while chosen.len() < card {
        let v = rng.below(nodes);
        if !chosen.contains(&v) {
            chosen.push(v);
        }
        guard += 1;
        if guard > card * 64 {
            // Degenerate RNG luck: fill deterministically with the missing ones.
            for v in 0..nodes {
                if chosen.len() == card {
                    break;
                }
                if !chosen.contains(&v) {
                    chosen.push(v);
                }
            }
            break;
        }
    }
    chosen
}

fn random_uniform(
    nodes: usize,
    edges: usize,
    card: usize,
    seed: u64,
) -> Result<Built, WorkloadError> {
    if nodes < 2 || edges == 0 || card < 1 {
        return Err(WorkloadError::InvalidParameter(
            "random uniform: degenerate",
        ));
    }
    if card > nodes {
        return Err(WorkloadError::InvalidParameter("card > nodes"));
    }
    let incidences = cmul(edges, card, "edges*card")?;
    let oracle = oracle_of(nodes, incidences, edges);
    super::guard_size(&oracle)?;

    let mut rng = SplitMix64::new(seed);
    let mut built = Vec::with_capacity(edges);
    for _ in 0..edges {
        built.push(distinct_sample(&mut rng, nodes, card));
    }
    finish_generic(&format!("random_uniform_s{seed}"), nodes, built, oracle)
}

fn random_degree(
    nodes: usize,
    edges: usize,
    min_card: usize,
    max_card: usize,
    seed: u64,
) -> Result<Built, WorkloadError> {
    if nodes < 2 || edges == 0 || min_card < 1 || max_card < min_card || max_card > nodes {
        return Err(WorkloadError::InvalidParameter("random degree: degenerate"));
    }
    // Controlled cardinality: edge j has card = min + (j % (max-min+1)).
    let span = max_card - min_card + 1;
    let mut incidences = 0usize;
    for j in 0..edges {
        incidences = cadd(incidences, min_card + (j % span), "sum cardinalities")?;
    }
    let oracle = oracle_of(nodes, incidences, edges);
    super::guard_size(&oracle)?;

    let mut rng = SplitMix64::new(seed);
    let mut built = Vec::with_capacity(edges);
    for j in 0..edges {
        let card = min_card + (j % span);
        built.push(distinct_sample(&mut rng, nodes, card));
    }
    finish_generic(&format!("random_degree_s{seed}"), nodes, built, oracle)
}

#[allow(clippy::too_many_arguments)]
fn random_hub(
    nodes: usize,
    edges: usize,
    hubs: usize,
    hub_ratio_num: u64,
    hub_ratio_den: u64,
    card: usize,
    seed: u64,
) -> Result<Built, WorkloadError> {
    if nodes < 2 || edges == 0 || card < 1 || card > nodes || hubs == 0 || hubs >= nodes {
        return Err(WorkloadError::InvalidParameter("random hub: degenerate"));
    }
    let incidences = cmul(edges, card, "edges*card")?;
    let oracle = oracle_of(nodes, incidences, edges);
    super::guard_size(&oracle)?;

    let mut rng = SplitMix64::new(seed);
    let mut built = Vec::with_capacity(edges);
    for _ in 0..edges {
        let mut chosen: Vec<usize> = Vec::with_capacity(card);
        let mut guard = 0usize;
        while chosen.len() < card {
            let v = if rng.chance(hub_ratio_num, hub_ratio_den) {
                rng.below(hubs)
            } else {
                hubs + rng.below(nodes - hubs)
            };
            if !chosen.contains(&v) {
                chosen.push(v);
            }
            guard += 1;
            if guard > card * 64 {
                for v in 0..nodes {
                    if chosen.len() == card {
                        break;
                    }
                    if !chosen.contains(&v) {
                        chosen.push(v);
                    }
                }
                break;
            }
        }
        built.push(chosen);
    }
    finish_generic(&format!("random_hub_s{seed}"), nodes, built, oracle)
}

// ── synthetic networks ─────────────────────────────────────────────────────

fn vlan(
    switches: usize,
    hosts_per_switch: usize,
    vlans: usize,
    power_domains: usize,
) -> Result<Built, WorkloadError> {
    if switches < 2 || hosts_per_switch < 1 || vlans == 0 || power_domains == 0 {
        return Err(WorkloadError::InvalidParameter("vlan: degenerate"));
    }
    let hosts = cmul(switches, hosts_per_switch, "switches*hosts_per_switch")?;
    if hosts < 2 * vlans || switches < 2 * power_domains {
        return Err(WorkloadError::InvalidParameter(
            "vlan: groups must be multiway (>=2 members each)",
        ));
    }
    let vertices = cadd(switches, hosts, "switches+hosts")?;
    let units = cadd(vlans, power_domains, "vlans+power_domains")?;
    // Each host is in exactly one VLAN, each switch in exactly one power domain.
    let incidences = cadd(hosts, switches, "hosts+switches")?;
    let oracle = oracle_of(vertices, incidences, units);
    super::guard_size(&oracle)?;

    // Vertex order: switches [0..switches), hosts [switches..vertices).
    let mut names: Vec<String> = Vec::with_capacity(vertices);
    for s in 0..switches {
        names.push(format!("switch{s}"));
    }
    for h in 0..hosts {
        names.push(format!("host{h}"));
    }
    let host_id = |h: usize| switches + h;

    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(units);
    let mut unit_names: Vec<String> = Vec::with_capacity(units);
    // VLAN hyperedges.
    for v in 0..vlans {
        let members: Vec<usize> = (0..hosts).filter(|h| h % vlans == v).map(host_id).collect();
        edges.push(members);
        unit_names.push(format!("vlan{v}"));
    }
    // Power/failure-domain hyperedges over switches.
    for d in 0..power_domains {
        let members: Vec<usize> = (0..switches).filter(|s| s % power_domains == d).collect();
        edges.push(members);
        unit_names.push(format!("power_domain{d}"));
    }
    finish_named(
        "network_vlan_failure_domain",
        edges,
        names,
        unit_names,
        oracle,
    )
}

fn service_dependency(
    services: usize,
    replicas: usize,
    shared_groups: usize,
    dep_card: usize,
    seed: u64,
) -> Result<Built, WorkloadError> {
    if services == 0 || replicas == 0 || shared_groups == 0 || dep_card == 0 {
        return Err(WorkloadError::InvalidParameter("service dep: degenerate"));
    }
    if dep_card > shared_groups {
        return Err(WorkloadError::InvalidParameter("dep_card > shared_groups"));
    }
    let instances = cmul(services, replicas, "services*replicas")?;
    let vertices = cadd(instances, shared_groups, "instances+shared_groups")?;
    let units = instances;
    let per_unit = cadd(1, dep_card, "1+dep_card")?;
    let incidences = cmul(units, per_unit, "units*(1+dep_card)")?;
    let oracle = oracle_of(vertices, incidences, units);
    super::guard_size(&oracle)?;

    // Vertex order: instances [0..instances), shared resources [instances..].
    let mut names: Vec<String> = Vec::with_capacity(vertices);
    for s in 0..services {
        for r in 0..replicas {
            names.push(format!("svc{s}_replica{r}"));
        }
    }
    let resource_kinds = ["database", "cache", "bus"];
    for g in 0..shared_groups {
        names.push(format!("{}{g}", resource_kinds[g % resource_kinds.len()]));
    }
    let res_id = |g: usize| instances + g;

    let mut rng = SplitMix64::new(seed);
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(units);
    let mut unit_names: Vec<String> = Vec::with_capacity(units);
    for i in 0..instances {
        // The instance produces itself and consumes dep_card distinct resources.
        let deps = distinct_sample(&mut rng, shared_groups, dep_card);
        let mut e = vec![i];
        e.extend(deps.into_iter().map(res_id));
        edges.push(e);
        unit_names.push(format!("dep{i}"));
    }
    finish_named(
        "network_service_dependency",
        edges,
        names,
        unit_names,
        oracle,
    )
}

// ── robotics ───────────────────────────────────────────────────────────────

fn humanoid(
    arm_joints: usize,
    leg_joints: usize,
    sensors_per_joint: usize,
    actuator_redundancy: usize,
) -> Result<Built, WorkloadError> {
    // Two arms + two legs, but arms and legs may have different joint counts, so
    // the shared limbed_robot (uniform joints) is not a clean fit; build a
    // four-limb robot where limbs 0,1 are arms and 2,3 are legs by padding to a
    // common joint count is wrong. Instead assemble directly.
    if arm_joints == 0 || leg_joints == 0 || sensors_per_joint == 0 || actuator_redundancy == 0 {
        return Err(WorkloadError::InvalidParameter("humanoid: degenerate"));
    }
    let limbs: [(&str, usize); 4] = [
        ("left_arm", arm_joints),
        ("right_arm", arm_joints),
        ("left_leg", leg_joints),
        ("right_leg", leg_joints),
    ];
    build_multilimb(
        "robot_humanoid",
        &limbs,
        sensors_per_joint,
        actuator_redundancy,
    )
}

fn hexapod(
    legs: usize,
    joints_per_leg: usize,
    sensors_per_joint: usize,
    actuator_redundancy: usize,
) -> Result<Built, WorkloadError> {
    if legs == 0 || joints_per_leg == 0 || sensors_per_joint == 0 || actuator_redundancy == 0 {
        return Err(WorkloadError::InvalidParameter("hexapod: degenerate"));
    }
    let labels: Vec<(String, usize)> = (0..legs)
        .map(|l| (format!("leg{l}"), joints_per_leg))
        .collect();
    let refs: Vec<(&str, usize)> = labels.iter().map(|(s, j)| (s.as_str(), *j)).collect();
    build_multilimb(
        "robot_hexapod",
        &refs,
        sensors_per_joint,
        actuator_redundancy,
    )
}

/// Multi-limb robot with per-limb joint counts (arms vs legs may differ).
fn build_multilimb(
    root: &str,
    limbs: &[(&str, usize)],
    sensors: usize,
    actuators: usize,
) -> Result<Built, WorkloadError> {
    let per_joint = 2 + sensors + actuators; // joint, controller, sensors, actuators
    let total_joints: usize = limbs.iter().map(|(_, j)| *j).sum();
    let limb_vertices = cmul(total_joints, per_joint, "joints*per_joint")?;
    let vertices = cadd(limb_vertices, 3, "+torso/power/comm")?;
    let units = cadd(cmul(total_joints, 2, "joints*2")?, 2, "+buses")?;
    let incidences = cadd(
        cadd(
            cmul(total_joints, per_joint, "local")?,
            cmul(total_joints, 2, "attach")?,
            "local+attach",
        )?,
        cadd(
            cadd(1, cmul(total_joints, actuators, "acts")?, "power")?,
            cadd(1, total_joints, "comm")?,
            "power+comm",
        )?,
        "total",
    )?;
    let oracle = oracle_of(vertices, incidences, units);
    super::guard_size(&oracle)?;

    let torso = 0usize;
    let power_bus = 1usize;
    let comm_bus = 2usize;
    let base = 3usize;
    let mut names: Vec<String> = vec!["torso".into(), "power_bus".into(), "comm_bus".into()];
    for (label, jn) in limbs {
        for j in 0..*jn {
            names.push(format!("{label}_joint{j}"));
            names.push(format!("{label}_controller{j}"));
            for s in 0..sensors {
                names.push(format!("{label}_sensor{j}_{s}"));
            }
            for a in 0..actuators {
                names.push(format!("{label}_actuator{j}_{a}"));
            }
        }
    }
    let anchor = |g: usize| base + g * per_joint;
    let joint_v = anchor;
    let controller_v = |g: usize| anchor(g) + 1;
    let sensor_v = |g: usize, s: usize| anchor(g) + 2 + s;
    let actuator_v = |g: usize, a: usize| anchor(g) + 2 + sensors + a;

    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(units);
    let mut unit_names: Vec<String> = Vec::with_capacity(units);
    for g in 0..total_joints {
        let mut e = vec![controller_v(g), joint_v(g)];
        for s in 0..sensors {
            e.push(sensor_v(g, s));
        }
        for a in 0..actuators {
            e.push(actuator_v(g, a));
        }
        edges.push(e);
        unit_names.push(format!("joint_local{g}"));
        edges.push(vec![torso, joint_v(g)]);
        unit_names.push(format!("joint_attach{g}"));
    }
    let mut pe = vec![power_bus];
    for g in 0..total_joints {
        for a in 0..actuators {
            pe.push(actuator_v(g, a));
        }
    }
    edges.push(pe);
    unit_names.push("power_bus_edge".into());
    let mut ce = vec![comm_bus];
    for g in 0..total_joints {
        ce.push(controller_v(g));
    }
    edges.push(ce);
    unit_names.push("comm_bus_edge".into());

    finish_named(root, edges, names, unit_names, oracle)
}

// ── synthetic biomedical (NOT scientifically validated) ────────────────────

fn chicken_physiology() -> Result<Built, WorkloadError> {
    // A fixed, deliberately small structural model. NOT a validated biological
    // model — a benchmark shaped like an organ/process network.
    let materials = [
        "beak",
        "crop",
        "gizzard",
        "small_intestine",
        "cloaca", // digestive
        "lung",
        "air_sac", // respiratory
        "heart",
        "blood", // circulatory
        "liver",
        "kidney", // metabolic
        "keel",
        "wing_muscle",
        "leg_muscle", // musculoskeletal
    ];
    let names: Vec<String> = materials.iter().map(|s| (*s).to_string()).collect();
    let idx = |n: &str| names.iter().position(|x| x == n).unwrap();

    // Process hyperedges (multiway, >2). First member produced, rest consumed.
    let procs: Vec<(&str, Vec<&str>)> = vec![
        (
            "digestion",
            vec!["small_intestine", "beak", "crop", "gizzard", "cloaca"],
        ),
        ("respiration", vec!["blood", "lung", "air_sac"]),
        (
            "circulation",
            vec!["blood", "heart", "lung", "liver", "kidney"],
        ),
        (
            "metabolism",
            vec!["blood", "liver", "kidney", "small_intestine"],
        ),
        ("locomotion", vec!["keel", "wing_muscle", "leg_muscle"]),
    ];
    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(procs.len());
    let mut unit_names: Vec<String> = Vec::with_capacity(procs.len());
    let mut incidences = 0usize;
    for (name, members) in &procs {
        edges.push(members.iter().map(|m| idx(m)).collect());
        unit_names.push((*name).to_string());
        incidences = cadd(incidences, members.len(), "incidences")?;
    }
    let oracle = oracle_of(names.len(), incidences, procs.len());
    finish_named("biomed_chicken_system", edges, names, unit_names, oracle)
}

fn chicken_flock(
    birds: usize,
    coops: usize,
    feeders: usize,
    drinkers: usize,
    group: usize,
    windows: usize,
    seed: u64,
) -> Result<Built, WorkloadError> {
    if birds < 2 || coops == 0 || feeders == 0 || drinkers == 0 || group < 2 || windows == 0 {
        return Err(WorkloadError::InvalidParameter("flock: degenerate"));
    }
    if birds < group {
        return Err(WorkloadError::InvalidParameter("group larger than flock"));
    }
    // Vertices: birds, then shared resources coops/feeders/drinkers.
    let shared = cadd(
        cadd(coops, feeders, "coops+feeders")?,
        drinkers,
        "+drinkers",
    )?;
    let vertices = cadd(birds, shared, "birds+shared")?;

    // Units:
    //   coop membership: each bird in coop (b%coops)      -> coops edges, sum=birds
    //   feeder sharing:  each bird uses feeder (b%feeders)-> feeders edges, sum=birds
    //   drinker sharing: each bird uses drinker(b%drink)  -> drinkers edges, sum=birds
    //   contact groups:  per window, `windows` groups of `group` random birds
    //                    -> windows edges, sum = windows*group
    let membership_units = cadd(
        cadd(coops, feeders, "coops+feeders")?,
        drinkers,
        "+drinkers",
    )?;
    let units = cadd(membership_units, windows, "+windows")?;
    // Each membership edge is {resource} + its birds; every bird is in exactly
    // one coop, one feeder group and one drinker group.
    let membership_inc = cadd(shared, cmul(3, birds, "3*birds")?, "shared+3*birds")?;
    let contact_inc = cmul(windows, group, "windows*group")?;
    let incidences = cadd(membership_inc, contact_inc, "membership+contact")?;
    let oracle = oracle_of(vertices, incidences, units);
    super::guard_size(&oracle)?;

    let mut names: Vec<String> = Vec::with_capacity(vertices);
    for b in 0..birds {
        names.push(format!("bird{b}"));
    }
    for c in 0..coops {
        names.push(format!("coop{c}"));
    }
    for f in 0..feeders {
        names.push(format!("feeder{f}"));
    }
    for d in 0..drinkers {
        names.push(format!("drinker{d}"));
    }
    let coop_v = |c: usize| birds + c;
    let feeder_v = |f: usize| birds + coops + f;
    let drinker_v = |d: usize| birds + coops + feeders + d;

    let mut edges: Vec<Vec<usize>> = Vec::with_capacity(units);
    let mut unit_names: Vec<String> = Vec::with_capacity(units);
    for c in 0..coops {
        let mut e = vec![coop_v(c)];
        e.extend((0..birds).filter(|b| b % coops == c));
        edges.push(e);
        unit_names.push(format!("coop_group{c}"));
    }
    for f in 0..feeders {
        let mut e = vec![feeder_v(f)];
        e.extend((0..birds).filter(|b| b % feeders == f));
        edges.push(e);
        unit_names.push(format!("feeder_group{f}"));
    }
    for d in 0..drinkers {
        let mut e = vec![drinker_v(d)];
        e.extend((0..birds).filter(|b| b % drinkers == d));
        edges.push(e);
        unit_names.push(format!("drinker_group{d}"));
    }
    let mut rng = SplitMix64::new(seed);
    for w in 0..windows {
        let members = distinct_sample(&mut rng, birds, group);
        edges.push(members);
        unit_names.push(format!("contact_w{w}"));
    }
    finish_named(
        "biomed_chicken_flock_contact",
        edges,
        names,
        unit_names,
        oracle,
    )
}
