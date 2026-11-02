//! Ordered compensated accumulation for PageRank.
use hymeko::ir::incidence::IncidenceAccumulator;

/// Neumaier accumulation for long nonnegative sums and L1 norms.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Sum {
    value: f64,
    correction: f64,
}
impl Sum {
    pub(super) fn add(&mut self, value: f64) {
        let next = self.value + value;
        self.correction += if self.value.abs() >= value.abs() {
            (self.value - next) + value
        } else {
            (value - next) + self.value
        };
        self.value = next;
    }
    pub(super) fn total(self) -> f64 {
        self.value + self.correction
    }
    pub(super) fn of(values: impl Iterator<Item = f64>) -> f64 {
        <Self as IncidenceAccumulator>::sum(values)
    }
}

impl IncidenceAccumulator for Sum {
    #[inline]
    fn sum_chunks<I, C>(chunks: I) -> f64
    where
        I: IntoIterator<Item = C>,
        C: IntoIterator<Item = f64>,
    {
        let mut state = None;
        for chunk in chunks {
            let mut terms = chunk.into_iter();
            let accumulator = match state {
                Some(accumulator) => accumulator,
                None => {
                    let Some(first) = terms.next() else { continue };
                    Self::from_first(first)
                }
            };
            state = Some(terms.fold(accumulator, |mut accumulator, term| {
                accumulator.add(term);
                accumulator
            }));
        }
        state.unwrap_or_default().total()
    }
    #[inline]
    fn sum_pair(first: impl FnOnce() -> f64, second: impl FnOnce() -> f64) -> f64 {
        let first = first();
        let second = second();
        let sum = first + second;
        if sum.is_finite() {
            // For two finite terms, the completed Neumaier reduction rounds
            // back to this same sum. Canonicalize zero as the final +correction
            // did; retain the general recurrence for overflow and nonfinites.
            sum + 0.0
        } else {
            Self::sum([first, second])
        }
    }
    #[inline]
    fn from_first(value: f64) -> Self {
        // The first Neumaier update has a known zero state. Keep the rounded
        // zero addition and error term: signed zero and nonfinite inputs retain
        // the same behavior as Default + add, without the magnitude branch.
        let rounded = 0.0 + value;
        Self {
            value: rounded,
            correction: (value - rounded) + 0.0,
        }
    }
    #[inline]
    fn add(&mut self, value: f64) {
        Sum::add(self, value);
    }
    #[inline]
    fn total(&self) -> f64 {
        Sum::total(*self)
    }
}
