"""Framework incidence exports composed with SciPy's GMRES, never native PageRank.

The adapter applies the normalized incidence factors through LinearOperator.
SciPy owns the iterative solver; no pairwise adjacency or second IR is built.
All native incidence export, multiplicity binding, normalization and solving
happen inside query(). Graph construction and independent checks are outside.
"""
from collections import Counter
from dataclasses import dataclass, field
import hashlib
import math
import time

import numpy as np
from scipy import sparse
from scipy.sparse.linalg import LinearOperator, gmres

from external import ComposedBackend, OperationLimit


@dataclass
class RankResult:
    scores: np.ndarray = field(compare=False)
    checksum: str
    iterations: int
    matvecs: int
    bound: float = field(compare=False)


class PhaseObserver:
    """Observe one query's existing boundaries; fake clocks support untimed tests."""
    names = ("incidence", "preparation", "solve", "cleanup")

    def __init__(self, clock=None):
        self.clock = clock if clock is not None else time.perf_counter_ns
        self.samples = []
        self.current = None
        self.previous = None

    def begin(self):
        if self.current is not None:
            raise ValueError("unfinished phase observation")
        self.current = []
        self.previous = self.clock()

    def mark(self, name):
        if self.current is None or name != self.names[len(self.current)]:
            raise ValueError("unexpected phase order")
        now = self.clock()
        if now < self.previous:
            raise ValueError("nonmonotonic phase clock")
        self.current.append(now-self.previous)
        self.previous = now
        if len(self.current) == len(self.names):
            self.samples.append(tuple(self.current))
            self.current = None

    def reset(self):
        if self.current is not None:
            raise ValueError("cannot discard an incomplete query")
        self.samples.clear()

    def means_us(self, repetitions):
        if self.current is not None or repetitions <= 0 or len(self.samples) != repetitions:
            raise ValueError("phase/sample count mismatch")
        return {name+"_us":sum(row[i] for row in self.samples)/repetitions/1000
                for i,name in enumerate(self.names)}


class RankingBackend(ComposedBackend):
    damping = .85
    tolerance = 1e-10
    max_iterations = 200

    def __init__(self, case, backend, observer=None):
        if any(len(set(edge)) != len(edge) for edge in case.edges):
            raise ValueError("ranking adapter requires set-valued memberships within each hyperedge")
        if float(case.data.get("damping", self.damping)) != self.damping:
            raise ValueError("reference damping does not match the ranking contract")
        super().__init__(case, backend)
        self.n = case.n
        self.has_incidence = bool(case.active)
        self.incidence_count = case.data["incidences"]
        self.detached_multiplicities = None
        self.reference = None
        self.verified_bound = self.reference_l1 = math.inf
        self.observer = observer

    def incidence(self):
        """Export native topology; canonical data supplies only declared multiplicity."""
        n, native = self.n, self.native.graph
        if not self.has_incidence:
            return sparse.csr_array((n, 0), dtype=float), np.empty(0)
        # HNX and XGI retain separate edge identities; only simple-hypergraph
        # containers need an explicit multiplicity map for merged hyperedges.
        multiplicities = ((self.detached_multiplicities if self.case is None else
                           Counter(tuple(sorted(edge)) for edge in self.case.edges))
                          if self.name in {"hypergraphx", "reticula"} else None)
        if self.name == "hypernetx":
            matrix, rows, _ = native.incidence_matrix(index=True, weights=False)
            weights = np.ones(matrix.shape[1])
        elif self.name == "xgi":
            matrix, rowmap, _ = self.native.module.incidence_matrix(native, sparse=True, index=True)
            rows = [rowmap[i] for i in range(matrix.shape[0])]
            weights = np.ones(matrix.shape[1])
        elif self.name == "hypergraphx":
            from hypergraphx.linalg.linalg import binary_incidence_matrix
            matrix, rowmap = binary_incidence_matrix(native, return_mapping=True)
            rows = [rowmap[i] for i in range(matrix.shape[0])]
            weights = np.asarray([multiplicities[tuple(sorted(edge))] for edge in native.get_edges()], dtype=float)
        else:
            edges = [tuple(map(int,edge.incident_verts())) for edge in native.edges()]
            columns = np.repeat(np.arange(len(edges)), [len(edge) for edge in edges])
            members = np.fromiter((v for edge in edges for v in edge), dtype=np.int64)
            matrix = sparse.csr_array((np.ones(len(members)), (members, columns)), shape=(n,len(edges)))
            rows = np.arange(n)
            weights = np.asarray([multiplicities[tuple(sorted(edge))] for edge in edges], dtype=float)
        rows = np.asarray(rows, dtype=np.int64)
        if len(rows) != matrix.shape[0] or len(np.unique(rows)) != len(rows) or np.any((rows < 0) | (rows >= n)):
            raise ValueError("invalid native incidence row mapping")
        matrix = sparse.csr_array(matrix, dtype=float)
        if len(rows) != n or not np.array_equal(rows, np.arange(n)):
            coo = matrix.tocoo()
            matrix = sparse.csr_array((coo.data,(rows[coo.row],coo.col)),shape=(n,matrix.shape[1]))
        if np.any(matrix.data != 1) or weights.shape != (matrix.shape[1],) or np.any(weights <= 0):
            raise ValueError("native incidence export or multiplicity mismatch")
        if float(np.asarray(matrix.sum(axis=0)).ravel() @ weights) != self.incidence_count:
            raise ValueError("native export changed the incidence multiplicity")
        return matrix, weights

    def detach_fixture(self):
        """Release oracle/input ownership; keep required multiedge multiplicity."""
        if self.case is not None:
            if self.name in {"hypergraphx", "reticula"}:
                self.detached_multiplicities = Counter(tuple(sorted(e)) for e in self.case.edges)
            self.case = self.native.case = None
            self.native.output = None
            self.reference = None

    def prepare(self, observer=None):
        matrix, weights = self.incidence()
        if observer is not None:
            observer.mark("incidence")
        n = self.n
        cardinality = np.asarray(matrix.sum(axis=0)).ravel()
        rates = np.divide(weights, cardinality, out=np.zeros_like(weights), where=cardinality > 0)
        degree = np.asarray(matrix @ weights).ravel()
        inverse = np.divide(1.0, degree, out=np.zeros_like(degree), where=degree > 0)
        transpose = matrix.T.tocsr()
        dangling = degree == 0
        prepared = PreparedRanking(matrix, transpose, rates, inverse, dangling,
                                   self.damping, self.tolerance, self.max_iterations)
        return prepared

    def query(self, operation="pagerank", materialize=True):
        """Return a full vector, with preparation included in the query boundary."""
        if operation != "pagerank":
            raise ValueError("ranking composition only supports pagerank")
        observer = self.observer
        if observer is not None:
            observer.begin()
        prepared = self.prepare(observer)
        if observer is not None:
            # Close after prepare() returns, including its temporary disposal.
            observer.mark("preparation")
        result = prepared.solve()
        if observer is not None:
            observer.mark("solve")
        del prepared
        if observer is not None:
            observer.mark("cleanup")
        return result

    def verify(self, result, operation="pagerank"):
        """Check the full vector against the canonical export, outside timing."""
        if operation != "pagerank" or result.scores.shape != (self.case.n,) or not np.isfinite(result.scores).all():
            raise ValueError("invalid full ranking result")
        scores, n = result.scores, self.case.n
        if n == 0:
            self.verified_bound = self.reference_l1 = 0.0
            return
        if self.reference is None:
            edges = self.case.edges
            columns = np.repeat(np.arange(len(edges)), [len(edge) for edge in edges])
            members = np.fromiter((v for edge in edges for v in edge),dtype=np.int64)
            self.reference = sparse.csr_array((np.ones(len(members)),(members,columns)),shape=(n,len(edges)))
        reference = self.reference
        degree = np.asarray(reference.sum(axis=1)).ravel()
        cardinality = np.asarray(reference.sum(axis=0)).ravel()
        sent = np.divide(scores,degree,out=np.zeros(n),where=degree > 0)
        gathered = np.asarray(reference.T @ sent).ravel()
        gathered = np.divide(gathered,cardinality,out=np.zeros_like(gathered),where=cardinality > 0)
        expected = self.damping*(reference @ gathered) + ((1-self.damping)+self.damping*scores[degree == 0].sum())/n
        self.verified_bound = float(np.abs(expected-scores).sum()/(1-self.damping))
        known = np.asarray(self.case.data.get("pagerank",[]),dtype=float)
        if known.shape != scores.shape or not np.isfinite(known).all():
            raise ValueError("complete canonical ranking reference is required")
        self.reference_l1 = float(np.abs(known-scores).sum())
        if (self.verified_bound > 1.01*self.tolerance or self.reference_l1 > 2.01*self.tolerance
                or abs(float(scores.sum())-1) > self.tolerance or scores.min() < -self.tolerance):
            raise ValueError("ranking differs from canonical topology/reference or residual contract")


@dataclass
class PreparedRanking:
    """Own the same sparse factors used by the existing composed query."""
    matrix: object
    transpose: object
    rates: np.ndarray
    inverse: np.ndarray
    dangling: np.ndarray
    damping: float
    tolerance: float
    max_iterations: int

    def solve(self):
        matrix, transpose = self.matrix, self.transpose
        rates, inverse, dangling = self.rates, self.inverse, self.dangling
        n = matrix.shape[0]
        if n == 0:
            return RankResult(np.empty(0), hashlib.sha256(b"").hexdigest(), 0, 0, 0.0)
        counters = [0, 0]

        def transition(vector):
            return matrix @ ((transpose @ (inverse * vector)) * rates) + vector[dangling].sum()/n

        def apply(vector):
            counters[1] += 1
            return vector - self.damping * transition(vector)

        def iteration(_residual):
            counters[0] += 1

        operator = LinearOperator((n,n), matvec=apply, dtype=np.float64)
        uniform = np.full(n, 1/n)
        # ||r||_1 <= sqrt(n)||r||_2; the contraction error is <= ||r||_1/(1-alpha).
        # The factor 1/4 leaves room for rounding and the independent post-check.
        atol = .25 * self.tolerance * (1-self.damping) / math.sqrt(n)
        scores, info = gmres(operator, (1-self.damping)*uniform, x0=uniform,
                             rtol=0.0, atol=atol, restart=min(20,n), maxiter=self.max_iterations,
                             callback=iteration, callback_type="legacy")
        if info != 0 or not np.isfinite(scores).all():
            raise OperationLimit("scipy_gmres_did_not_converge_within_200_inner_iterations")
        residual = self.damping*transition(scores)+(1-self.damping)*uniform-scores
        bound = float(np.abs(residual).sum()/(1-self.damping))
        if bound > self.tolerance or scores.min() < -self.tolerance or abs(float(scores.sum())-1) > self.tolerance:
            raise ValueError("ranking solver violated the global probability/error contract")
        checksum = hashlib.sha256(np.asarray(scores,dtype="<f8").tobytes()).hexdigest()
        return RankResult(scores,checksum,*counters,bound)
