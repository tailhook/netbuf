use std::ops::{Range, RangeFrom, RangeTo, RangeFull};

/// Temporary type until the one in stdlib is made stable
pub enum RangeArgument {
    RangeFrom(usize),
    Range(usize, usize),
    RangeTo(usize),
}

impl From<Range<usize>> for RangeArgument {
    fn from(r: Range<usize>) -> RangeArgument {
        RangeArgument::Range(r.start, r.end)
    }
}
impl From<RangeFrom<usize>> for RangeArgument {
    fn from(r: RangeFrom<usize>) -> RangeArgument {
        RangeArgument::RangeFrom(r.start)
    }
}
impl From<RangeTo<usize>> for RangeArgument {
    fn from(r: RangeTo<usize>) -> RangeArgument {
        RangeArgument::RangeTo(r.end)
    }
}
impl From<RangeFull> for RangeArgument {
    fn from(_: RangeFull) -> RangeArgument {
        RangeArgument::RangeFrom(0)
    }
}

