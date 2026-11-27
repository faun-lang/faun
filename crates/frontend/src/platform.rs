//! What the frontend reads of the target a program is compiled for.
//!
//! A target says a great deal more -- how wide its word is, what each
//! routine reads and leaves in a machine's terms, how its image is written
//! -- and all of that is the tiers below Barb's to read. The frontend reads
//! four things: which host module says what `Host` offers, which routines
//! the target answers for, which of them use up what they are handed, and
//! which counts of a word's bits it answers with an instruction, which is
//! what Barb keeps whole rather than inlines.
//! Whoever drives the compile says those, from the target it chose.

/// What the frontend reads of a target.
#[derive(Clone, Debug)]
pub struct Platform {
    name: String,
    routines: Vec<Routine>,
    counts: barb::Counts,
}

/// A routine a target answers for, as the frontend reads one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Routine {
    /// The one name it has: what a program writes, and what the target is
    /// asked by.
    pub name: String,
    /// Whether it uses up the first argument it is handed: a resource, such
    /// as an open file, of which nothing may be asked afterwards. A type
    /// some routine uses up is a resource, which a program must use up
    /// exactly once.
    pub consumes: bool,
}

impl Platform {
    /// The target named `name`, whose host module is `hosts/<name>.fn`
    /// beside the standard package, answering for `routines`.
    #[must_use]
    pub fn new(name: impl Into<String>, routines: impl IntoIterator<Item = Routine>) -> Self {
        Self {
            name: name.into(),
            routines: routines.into_iter().collect(),
            counts: barb::Counts::NONE,
        }
    }

    /// The same, answering `counts` of a word's bits with instructions of
    /// its own: see [`barb::Program::answer_counts`].
    #[must_use]
    pub const fn with_counts(mut self, counts: barb::Counts) -> Self {
        self.counts = counts;
        self
    }

    /// Which counts of a word's bits it answers with instructions.
    #[must_use]
    pub const fn counts(&self) -> barb::Counts {
        self.counts
    }

    /// What names the target, and its host module.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The routine the target answers to by `name`.
    #[must_use]
    pub fn routine(&self, name: &str) -> Option<&Routine> {
        self.routines.iter().find(|routine| routine.name == name)
    }
}
