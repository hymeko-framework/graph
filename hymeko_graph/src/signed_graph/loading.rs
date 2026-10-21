use super::SignedGraph;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

/// Stream-based graph loading. Implementations own format and validation policy.
/// This loads edge-list data; HyMeKo source loading remains parser/Core-owned.
pub trait Loader {
    /// Graph representation produced by this format.
    type Output;
    /// Consume a buffered stream without retaining its complete contents.
    /// # Errors
    /// I/O errors and the implementation's format/domain errors.
    fn load(&self, reader: impl BufRead) -> io::Result<Self::Output>;

    /// Open a file and delegate to the same stream implementation.
    /// # Errors
    /// File-open errors and errors documented by `load`.
    fn load_path(&self, path: impl AsRef<Path>) -> io::Result<Self::Output> {
        self.load(BufReader::new(File::open(path)?))
    }
}

/// Compatibility loader for the signed `u v sign` edge-list examples.
///
/// Trims each line, skips empty/comment/malformed rows and self loops, and splits
/// on individual space, tab or comma characters (repeated separators yield empty
/// fields, as in the original examples). Extra fields are ignored. Signs parse
/// as i8 and are preserved without normalization; duplicates/orientation are kept.
/// Empty input produces one isolated vertex, matching the original convention.
/// Reuses one line buffer and writes directly to the final edge/sign vectors.
#[derive(Clone, Copy, Default, Debug)]
pub struct SignedEdgeListLoader;

impl SignedEdgeListLoader {
    fn edge(line: &str) -> Option<(u32, u32, i8)> {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut fields = line.split([' ', '\t', ',']);
        let u = fields.next()?.parse().ok()?;
        let v = fields.next()?.parse().ok()?;
        let sign = fields.next()?.parse().ok()?;
        (u != v).then_some((u, v, sign))
    }
}

impl Loader for SignedEdgeListLoader {
    type Output = SignedGraph;

    fn load(&self, mut reader: impl BufRead) -> io::Result<SignedGraph> {
        let mut graph = SignedGraph {
            n_nodes: 1,
            edges: Vec::new(),
            signs: Vec::new(),
        };
        let mut line = String::new();
        loop {
            line.clear();
            if reader.read_line(&mut line)? == 0 {
                return Ok(graph);
            }
            if let Some((u, v, sign)) = Self::edge(&line) {
                let nodes = u.max(v).checked_add(1).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "vertex count exceeds u32")
                })?;
                graph.n_nodes = graph.n_nodes.max(nodes);
                graph.edges.push((u, v));
                graph.signs.push(sign);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn compatibility_rows_preserve_order_signs_and_separator_rules() {
        let data =
            b" # comment\r\n0 1 1\n1\t2\t-1\n2,0,0,ignored\n2 2 -1\nbad\n3  4 1\n4 5 128\n1 0 1";
        let graph = SignedEdgeListLoader.load(Cursor::new(data)).unwrap();
        assert_eq!(graph.n_nodes, 3);
        assert_eq!(graph.edges, [(0, 1), (1, 2), (2, 0), (1, 0)]);
        assert_eq!(graph.signs, [1, -1, 0, 1]);
        assert_eq!(
            SignedEdgeListLoader.load(Cursor::new([])).unwrap().n_nodes,
            1
        );
    }

    #[test]
    fn unrepresentable_vertex_and_invalid_utf8_return_errors() {
        assert_eq!(
            SignedEdgeListLoader
                .load(Cursor::new(b"4294967295 0 1"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            SignedEdgeListLoader
                .load(Cursor::new([255]))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn file_loading_and_stream_error_are_observable() {
        // The committed source itself contains no valid numeric edge-list rows.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/signed_graph/loading.rs");
        assert!(
            SignedEdgeListLoader
                .load_path(&path)
                .unwrap()
                .edges
                .is_empty()
        );
        assert!(
            SignedEdgeListLoader
                .load_path(path.join("missing"))
                .is_err()
        );
        struct Failing;
        impl io::Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected"))
            }
        }
        let error = SignedEdgeListLoader
            .load(BufReader::new(Failing))
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }
}
