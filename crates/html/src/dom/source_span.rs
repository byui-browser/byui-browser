#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceSpan {
    start: usize,
    end: usize,
}

impl SourceSpan {
    /// Constructs SourceSpan, sets immutable start and end.
    pub fn new(start: usize, end: usize) -> Self {
        SourceSpan { start, end }
    }

    /// Gets start byte.
    pub fn get_start(&self) -> usize {
        self.start
    }
    /// Gets end byte.
    pub fn get_end(&self) -> usize {
        self.end
    }
}
