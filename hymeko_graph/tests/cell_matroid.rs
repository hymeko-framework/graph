//! Graded-cell adapter checks against closed-form incidence graph families.
use hymeko_graph::{
    GraphicMatroidWorkspace, MatroidError,
    cell_topology::CellTopology,
    matroid::{CellLeviCoord, CellLeviMatroid},
};
struct Cells;
impl Cells {
    fn tetrahedron() -> CellTopology {
        CellTopology::vertices(5)
            .with_rank(
                2,
                vec![0, 1, 0, 2, 0, 3, 1, 2, 1, 3, 2, 3],
                [-1.0, 1.0].repeat(6),
            )
            .unwrap()
            .with_rank(
                3,
                vec![0, 3, 1, 0, 4, 2, 1, 5, 2, 3, 5, 4],
                [1.0, 1.0, -1.0].repeat(4),
            )
            .unwrap()
            .with_rank(4, vec![0, 1, 2, 3], vec![1.0, -1.0, 1.0, -1.0])
            .unwrap()
    }
    fn triangle() -> CellTopology {
        CellTopology::vertices(3)
            .with_rank(2, vec![0, 1, 1, 2, 2, 0], [-1.0, 1.0].repeat(3))
            .unwrap()
    }
    fn triangle_coords() -> Vec<CellLeviCoord> {
        (0..3)
            .flat_map(|cell| {
                (0..2).map(move |slot| CellLeviCoord {
                    rank: 1,
                    cell,
                    slot,
                })
            })
            .collect()
    }
}
#[test]
fn graded_tetrahedron_includes_all_ranks_and_isolated_cells() {
    let topology = Cells::tetrahedron();
    let matroid = CellLeviMatroid::new(&topology).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    // 5 vertices + 6 edges + 4 faces + 1 volume; one isolated vertex.
    assert_eq!(matroid.vertex_count(), 16);
    assert_eq!(matroid.components(&mut ws).unwrap(), 2);
    assert_eq!(matroid.rank(&mut ws).unwrap(), 14);
    let mut basis = Vec::new();
    assert_eq!(matroid.spanning_basis(&mut ws, &mut basis).unwrap(), 14);
    assert!(matroid.independent(&basis, &mut ws).unwrap());
    assert!(basis.iter().any(|c| c.rank == 3));
    let saved = basis.clone();
    matroid.spanning_basis(&mut ws, &mut basis).unwrap();
    assert_eq!(basis, saved);
    let mut session = matroid.incremental(&mut ws).unwrap();
    for &coordinate in &basis {
        assert!(session.offer(coordinate).unwrap());
    }
    assert_eq!(session.accepted(), 14);
    assert!(!session.offer(basis[0]).unwrap());
}
#[test]
fn triangle_incidence_cycle_obeys_heredity_and_exchange() {
    let topology = Cells::triangle();
    let m = CellLeviMatroid::new(&topology).unwrap();
    let coordinates = Cells::triangle_coords();
    let mut ws = GraphicMatroidWorkspace::new();
    let mut independent = [false; 64];
    for (set, flag) in independent.iter_mut().enumerate() {
        let subset: Vec<_> = coordinates
            .iter()
            .enumerate()
            .filter_map(|(i, &c)| (set & (1 << i) != 0).then_some(c))
            .collect();
        *flag = m.independent(&subset, &mut ws).unwrap();
        // The Levi graph is one simple six-edge cycle: only the whole cycle is dependent.
        assert_eq!(*flag, set != 63);
    }
    for a in 0usize..64 {
        if independent[a] {
            for i in 0..6 {
                if a & (1 << i) != 0 {
                    assert!(independent[a ^ (1 << i)]);
                }
            }
            for b in 0usize..64 {
                if independent[b] && a.count_ones() < b.count_ones() {
                    assert!((0..6).any(|i| b & (1 << i) != 0
                        && a & (1 << i) == 0
                        && independent[a | (1 << i)]));
                }
            }
        }
    }
    let mut session = m.incremental(&mut ws).unwrap();
    for (i, c) in coordinates.into_iter().enumerate() {
        assert_eq!(session.offer(c).unwrap(), i < 5);
    }
    assert_eq!(session.accepted(), 5);
}
#[test]
fn parallel_slots_zero_coefficients_and_invalid_coordinates_are_preserved() {
    for signs in [vec![1.0, -1.0], vec![0.0, 0.5]] {
        let topology = CellTopology::vertices(2)
            .with_rank(2, vec![0, 0], signs)
            .unwrap();
        let m = CellLeviMatroid::new(&topology).unwrap();
        let mut ws = GraphicMatroidWorkspace::new();
        assert_eq!(m.rank(&mut ws).unwrap(), 1);
        assert_eq!(m.components(&mut ws).unwrap(), 2);
        let coords = [
            CellLeviCoord {
                rank: 1,
                cell: 0,
                slot: 0,
            },
            CellLeviCoord {
                rank: 1,
                cell: 0,
                slot: 1,
            },
        ];
        assert!(!m.independent(&coords, &mut ws).unwrap());
        for c in [
            CellLeviCoord {
                rank: 0,
                cell: 0,
                slot: 0,
            },
            CellLeviCoord {
                rank: 2,
                cell: 0,
                slot: 0,
            },
            CellLeviCoord {
                rank: 1,
                cell: 1,
                slot: 0,
            },
            CellLeviCoord {
                rank: 1,
                cell: 0,
                slot: 2,
            },
        ] {
            assert!(matches!(
                m.independent(&[c], &mut ws),
                Err(MatroidError::CellIncidenceOutOfRange { .. })
            ));
        }
    }
}
#[test]
fn empty_and_overwide_cell_universes_are_checked_without_graph_materialization() {
    let empty = CellTopology::vertices(0);
    let m = CellLeviMatroid::new(&empty).unwrap();
    let mut ws = GraphicMatroidWorkspace::new();
    assert_eq!(m.rank(&mut ws).unwrap(), 0);
    assert_eq!(m.components(&mut ws).unwrap(), 0);
    let isolated = CellTopology::vertices(7);
    assert_eq!(
        CellLeviMatroid::new(&isolated)
            .unwrap()
            .components(&mut ws)
            .unwrap(),
        7
    );
    if let Ok(count) = usize::try_from(u64::from(u32::MAX) + 1) {
        let overwide = CellTopology::vertices(count);
        assert!(matches!(
            CellLeviMatroid::new(&overwide),
            Err(MatroidError::Overflow)
        ));
    }
}
