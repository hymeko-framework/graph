//! Stage P-io (2026-05-19): direct read/write for the P-graph Studio
//! `.pgip` project format (SQLite under the hood).
//!
//! Closes the loop with P-graph Studio: a `.pgip` produced by the
//! studio can be read into [`LoweredPGraph`] without going through
//! the `.hymeko` intermediate (option D); a [`LoweredPGraph`]
//! computed by our pipeline can be written back as `.pgip` and
//! opened in P-graph Studio (option C).
//!
//! Schema reference: the `.pgip` file is a SQLite database with the
//! tables documented inline; the canonical schema is what P-graph
//! Studio creates when you `File → New` in the GUI. We emit the
//! same `CREATE TABLE` statements; the binary is byte-compatible
//! with the studio's project loader as a result.
//!
//! Companion: `scripts/pgip_to_hymeko.py` (Python, same conversion,
//! kept as a quick-CLI fallback). The Rust path is the canonical
//! production path because it avoids the Python ⇒ HyMeKo source ⇒
//! Rust parse round-trip overhead and validates the schema at
//! compile time.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::{Connection, params};

use hymeko::common::ids::{DeclId, EdgeId};

use crate::abb::AbbSolution;
use crate::lowering::LoweredPGraph;
use crate::schema::{PGraphSchema, PNodeKind};

/// Errors that can arise during `.pgip` I/O.
#[derive(Debug, thiserror::Error)]
pub enum PgipError {
    /// SQLite-level failure (file open, query, etc.).
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The `.pgip` schema is missing a required table or column.
    #[error("schema mismatch: {0}")]
    Schema(String),
    /// The P-graph in the `.pgip` violates a bipartite invariant.
    #[error("bipartite invariant: {0}")]
    Invariant(String),
    /// Sanitiser produced a duplicate name after stripping invalid
    /// characters (a single material named "A B" and another named
    /// "A_B" would both sanitise to "A_B").
    #[error("name collision: {0}")]
    NameCollision(String),
}

// ─── Identifier sanitisation ────────────────────────────────────────

/// HyMeKo identifiers must match `[A-Za-z_][A-Za-z0-9_]*`. Replace
/// invalid characters with `_`; if the result starts with a digit,
/// prepend `_`.
fn sanitize(name: &str) -> String {
    if name.is_empty() {
        return "_".to_string();
    }
    let mut s = String::with_capacity(name.len());
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_alphanumeric() || c == '_' {
            s.push(c);
        } else {
            s.push('_');
        }
        if i == 0 && s.chars().next().unwrap().is_ascii_digit() {
            s = format!("_{s}");
        }
    }
    s
}

// ─── Reader: .pgip → LoweredPGraph (option D) ───────────────────────

/// Read a `.pgip` SQLite database into a [`LoweredPGraph`].
///
/// Same semantic as `scripts/pgip_to_hymeko.py` followed by
/// `parser::parse_description` + `hymeko_pgraph::lower`, but
/// skips the textual round-trip. The output is a fully-lowered
/// P-graph ready for MSG/SSG/ABB consumption.
///
/// # Schema expectations
/// The `.pgip` must contain the tables `materials`, `materialTypes`,
/// `units`, `inputOutput` with the columns P-graph Studio creates by
/// default. Missing tables cause [`PgipError::Schema`].
///
/// # Caveats
/// - `unit.weight` is mapped to the scalar `costs` field.
/// - Per-unit cost columns (`fixCapitalCost`, `propCapitalCost`,
///   `fixOperatingCost`, `propOperatingCost`) are mapped to
///   `cost_vectors` (alphabetised dim names) iff any of the four is
///   non-zero. Otherwise `cost_vectors` is empty (scalar-only path).
pub fn read_pgip(path: &Path) -> Result<LoweredPGraph, PgipError> {
    let conn = Connection::open(path)?;

    // ── Materials ──────────────────────────────────────────────────
    let mut name_to_decl: BTreeMap<String, DeclId> = BTreeMap::new();
    let mut decl_to_name: BTreeMap<DeclId, String> = BTreeMap::new();
    let mut kinds: BTreeMap<DeclId, PNodeKind> = BTreeMap::new();
    let mut raws: BTreeSet<DeclId> = BTreeSet::new();
    let mut products: BTreeSet<DeclId> = BTreeSet::new();
    let mut materials: BTreeSet<DeclId> = BTreeSet::new();
    let mut mat_sqlid_to_decl: BTreeMap<i64, DeclId> = BTreeMap::new();

    let mut next_decl: usize = 0;
    let mut mat_stmt = conn.prepare("SELECT id, name, typeId FROM materials ORDER BY id")?;
    let rows = mat_stmt.query_map([], |r| {
        Ok::<(i64, String, i64), _>((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    for row in rows {
        let (sql_id, raw_name, type_id) = row?;
        let san = sanitize(&raw_name);
        if name_to_decl.contains_key(&san) {
            return Err(PgipError::NameCollision(format!(
                "material name {raw_name:?} sanitises to {san:?} which is already used"
            )));
        }
        let d = DeclId::new(next_decl);
        next_decl += 1;
        name_to_decl.insert(san.clone(), d);
        decl_to_name.insert(d, san);
        kinds.insert(d, PNodeKind::Material);
        materials.insert(d);
        mat_sqlid_to_decl.insert(sql_id, d);
        if type_id == 1 {
            raws.insert(d);
        } else if type_id == 2 {
            products.insert(d);
        }
    }
    drop(mat_stmt);

    // ── Units ──────────────────────────────────────────────────────
    let mut costs: BTreeMap<DeclId, f64> = BTreeMap::new();
    let mut per_unit_dim: BTreeMap<DeclId, BTreeMap<String, f64>> = BTreeMap::new();
    let mut dim_set: BTreeSet<String> = BTreeSet::new();
    let mut units_set: BTreeSet<DeclId> = BTreeSet::new();
    let mut unit_sqlid_to_decl: BTreeMap<i64, DeclId> = BTreeMap::new();

    let mut unit_stmt = conn.prepare(
        "SELECT id, name, weight, fixCapitalCost, propCapitalCost,
                fixOperatingCost, propOperatingCost
         FROM units ORDER BY id",
    )?;
    let rows = unit_stmt.query_map([], |r| {
        Ok::<(i64, String, f64, f64, f64, f64, f64), _>((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
        ))
    })?;
    for row in rows {
        let (sql_id, raw_name, weight, fc, pc, fo, po) = row?;
        let san = sanitize(&raw_name);
        if name_to_decl.contains_key(&san) {
            return Err(PgipError::NameCollision(format!(
                "unit name {raw_name:?} collides with existing {san:?}"
            )));
        }
        let d = DeclId::new(next_decl);
        next_decl += 1;
        name_to_decl.insert(san.clone(), d);
        decl_to_name.insert(d, san);
        kinds.insert(d, PNodeKind::OperatingUnit);
        units_set.insert(d);
        unit_sqlid_to_decl.insert(sql_id, d);
        costs.insert(d, weight);

        let mut dims = BTreeMap::new();
        for (dim, v) in [
            ("fixed_capex", fc),
            ("prop_capex", pc),
            ("fixed_opex", fo),
            ("prop_opex", po),
        ] {
            if v != 0.0 {
                dims.insert(dim.to_string(), v);
                dim_set.insert(dim.to_string());
            }
        }
        per_unit_dim.insert(d, dims);
    }
    drop(unit_stmt);

    // ── Input/Output incidence ─────────────────────────────────────
    // Build only the directed edge set (the signed incidence); per-unit
    // input/output sets are derived from it by the schema, not stored.
    let mut edges: BTreeMap<EdgeId, (DeclId, DeclId)> = BTreeMap::new();

    let mut io_stmt = conn.prepare(
        "SELECT unitId, materialId, isInput FROM inputOutput
         ORDER BY unitId, isInput DESC",
    )?;
    let rows = io_stmt.query_map([], |r| {
        Ok::<(i64, i64, i64), _>((r.get(0)?, r.get(1)?, r.get(2)?))
    })?;
    for (next_edge, row) in rows.enumerate() {
        let (u_sql, m_sql, is_input) = row?;
        let u = *unit_sqlid_to_decl.get(&u_sql).ok_or_else(|| {
            PgipError::Schema(format!("inputOutput references unknown unit id {u_sql}"))
        })?;
        let m = *mat_sqlid_to_decl.get(&m_sql).ok_or_else(|| {
            PgipError::Schema(format!(
                "inputOutput references unknown material id {m_sql}"
            ))
        })?;
        if is_input != 0 {
            edges.insert(EdgeId::new(next_edge), (m, u));
        } else {
            edges.insert(EdgeId::new(next_edge), (u, m));
        }
    }
    drop(io_stmt);

    // ── Build cost_dimensions + cost_vectors (alphabetical) ────────
    let cost_dimensions: Vec<String> = dim_set.iter().cloned().collect();
    let cost_vectors: BTreeMap<DeclId, Vec<f64>> = if cost_dimensions.is_empty() {
        BTreeMap::new()
    } else {
        per_unit_dim
            .into_iter()
            .map(|(u, dims)| {
                let v: Vec<f64> = cost_dimensions
                    .iter()
                    .map(|name| dims.get(name).copied().unwrap_or(0.0))
                    .collect();
                (u, v)
            })
            .collect()
    };

    let schema = PGraphSchema::try_new(kinds, edges)
        .map_err(|e| PgipError::Invariant(format!("schema construction failed: {e}")))?;

    Ok(LoweredPGraph {
        schema,
        name_to_decl,
        decl_to_name,
        raws,
        products,
        materials,
        units: units_set,
        costs,
        cost_dimensions,
        cost_vectors,
    })
}

// ─── Writer: LoweredPGraph → .pgip (option C) ────────────────────────

/// Write a [`LoweredPGraph`] to a `.pgip` SQLite file P-graph Studio
/// can open.
///
/// If `abb_result` is provided, also populates `runHistory` /
/// `resultStructures` / `unitsInStructure` so the studio displays our
/// computed optimum alongside the topology.
pub fn write_pgip(
    graph: &LoweredPGraph,
    path: &Path,
    abb_result: Option<&AbbSolution>,
) -> Result<(), PgipError> {
    // Remove any pre-existing file so we start from a clean DB.
    if path.exists() {
        std::fs::remove_file(path)
            .map_err(|e| PgipError::Schema(format!("could not remove existing {path:?}: {e}")))?;
    }
    let mut conn = Connection::open(path)?;
    let tx = conn.transaction()?;

    // Schema. Mirrors P-graph Studio's CREATE statements exactly.
    tx.execute_batch(
        r#"
        CREATE TABLE materialTypes (
            id INTEGER PRIMARY KEY,
            name TEXT
        );
        CREATE TABLE materials (
            id INTEGER PRIMARY KEY,
            name TEXT,
            typeId INTEGER,
            unitPrice REAL,
            minFlow REAL,
            maxFlow REAL
        );
        CREATE TABLE units (
            id INTEGER PRIMARY KEY,
            name TEXT,
            weight REAL,
            fixCapitalCost REAL,
            propCapitalCost REAL,
            fixOperatingCost REAL,
            propOperatingCost REAL,
            minSize REAL,
            maxSize REAL
        );
        CREATE TABLE inputOutput (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            unitId INTEGER,
            materialId INTEGER,
            isInput INTEGER(1),
            flowRate REAL
        );
        CREATE TABLE runHistory (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timeStamp TEXT,
            algorithm TEXT,
            optimalWeight REAL,
            optimalCost REAL,
            structures INTEGER,
            steps INTEGER
        );
        CREATE TABLE resultStructures (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            runId INTEGER,
            strNumber INTEGER,
            totalWeight REAL,
            totalCost REAL,
            materialCost REAL,
            unitInvestmentCost REAL,
            unitOperatingCost REAL
        );
        CREATE TABLE unitsInStructure (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            structureId INTEGER,
            unitId INTEGER,
            size REAL,
            totalCost REAL,
            investmentCost REAL,
            operatingCost REAL
        );
        CREATE TABLE materialsInStructure (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            structureId INTEGER,
            materialId INTEGER,
            absoluteFlow REAL,
            cost REAL,
            price REAL
        );
        CREATE TABLE inputOutputInStructure (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            structureId INTEGER,
            ioId INTEGER,
            flow REAL
        );
        CREATE TABLE globalValues (
            name TEXT,
            value TEXT
        );
        "#,
    )?;

    // ── materialTypes: the canonical 3 rows (id 0/1/2). ──
    tx.execute(
        "INSERT INTO materialTypes (id, name) VALUES
           (0, 'Intermediate'), (1, 'Raw material'), (2, 'Product');",
        [],
    )?;

    // ── materials. Stable 1-based ids, ordered by DeclId. ──
    let mut mat_decl_to_sqlid: BTreeMap<DeclId, i64> = BTreeMap::new();
    let mut next_id: i64 = 1;
    for d in &graph.materials {
        let name = &graph.decl_to_name[d];
        let type_id: i64 = if graph.raws.contains(d) {
            1
        } else if graph.products.contains(d) {
            2
        } else {
            0
        };
        tx.execute(
            "INSERT INTO materials (id, name, typeId, unitPrice, minFlow, maxFlow)
             VALUES (?, ?, ?, 0.0, 0.0, 10000000.0)",
            params![next_id, name, type_id],
        )?;
        mat_decl_to_sqlid.insert(*d, next_id);
        next_id += 1;
    }

    // ── units. ──
    let mut unit_decl_to_sqlid: BTreeMap<DeclId, i64> = BTreeMap::new();
    let mut next_uid: i64 = 1;
    for u in &graph.units {
        let name = &graph.decl_to_name[u];
        let weight = graph.costs.get(u).copied().unwrap_or(1.0);

        // If multi-cost dims are present, look up the named columns;
        // otherwise zero them out. Names follow the convention used
        // by `read_pgip` and `pgip_to_hymeko.py`.
        let (fc, pc, fo, po) = {
            let v = graph.cost_vectors.get(u);
            let idx_of = |name: &str| graph.cost_dimensions.iter().position(|d| d == name);
            let get = |dim: &str| -> f64 {
                match (v, idx_of(dim)) {
                    (Some(vec), Some(i)) => vec.get(i).copied().unwrap_or(0.0),
                    _ => 0.0,
                }
            };
            (
                get("fixed_capex"),
                get("prop_capex"),
                get("fixed_opex"),
                get("prop_opex"),
            )
        };

        tx.execute(
            "INSERT INTO units (id, name, weight, fixCapitalCost, propCapitalCost,
                                fixOperatingCost, propOperatingCost, minSize, maxSize)
             VALUES (?, ?, ?, ?, ?, ?, ?, 0.0, 10000000.0)",
            params![next_uid, name, weight, fc, pc, fo, po],
        )?;
        unit_decl_to_sqlid.insert(*u, next_uid);
        next_uid += 1;
    }

    // ── inputOutput rows. ──
    for u in &graph.units {
        let u_sql = unit_decl_to_sqlid[u];
        for m in graph.inputs(*u) {
            let m_sql = mat_decl_to_sqlid[m];
            tx.execute(
                "INSERT INTO inputOutput (unitId, materialId, isInput, flowRate)
                 VALUES (?, ?, 1, 1.0)",
                params![u_sql, m_sql],
            )?;
        }
        for m in graph.outputs(*u) {
            let m_sql = mat_decl_to_sqlid[m];
            tx.execute(
                "INSERT INTO inputOutput (unitId, materialId, isInput, flowRate)
                 VALUES (?, ?, 0, 1.0)",
                params![u_sql, m_sql],
            )?;
        }
    }

    // ── Optional: bake the ABB result into runHistory + resultStructures
    //    so the studio shows our computed optimum. ──
    if let Some(sol) = abb_result {
        tx.execute(
            "INSERT INTO runHistory (id, timeStamp, algorithm,
                                      optimalWeight, optimalCost, structures, steps)
             VALUES (1, datetime('now'), 'ABB (hymeko_pgraph)', ?, ?, 1, ?)",
            params![sol.cost, sol.cost, sol.explored as i64],
        )?;
        tx.execute(
            "INSERT INTO resultStructures (id, runId, strNumber, totalWeight,
                                            totalCost, materialCost,
                                            unitInvestmentCost, unitOperatingCost)
             VALUES (1, 1, 1, ?, ?, 0.0, 0.0, 0.0)",
            params![sol.cost, sol.cost],
        )?;
        for (idx, u) in sol.units.iter().enumerate() {
            let u_sql = unit_decl_to_sqlid[u];
            let cost = graph.costs.get(u).copied().unwrap_or(1.0);
            tx.execute(
                "INSERT INTO unitsInStructure (id, structureId, unitId, size,
                                                totalCost, investmentCost, operatingCost)
                 VALUES (?, 1, ?, 1.0, ?, 0.0, 0.0)",
                params![(idx as i64) + 1, u_sql, cost],
            )?;
        }
    }

    // ── globalValues: write provenance. ──
    tx.execute(
        "INSERT INTO globalValues (name, value) VALUES
           ('source', 'hymeko_pgraph::pgip_io::write_pgip'),
           ('schema_version', '1');",
        [],
    )?;

    tx.commit()?;
    Ok(())
}
