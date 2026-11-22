//! An ordered list of local rewrite rules, and the driver trying them.
//!
//! A rule looks at one site -- a call, a match -- and either declines or
//! says what the site is instead. Its guard is whatever the rule asks of
//! the walk it runs in before it matches: which switches are on, what a
//! local was written as, what is recognised of a definition. Rules are
//! tried in the order they are listed, the first to fire wins, and what it
//! gives is either final or another site of the same kind, which the rules
//! are tried on again from the top. That is the whole engine: a list, a
//! loop, and fuel.
//!
//! What a rewrite changes beyond its own site needs no worklist. Barb's
//! terms are in A-normal form and rewritten front to back, so everything
//! that reads a rewritten binding -- its parents -- is visited after it,
//! and sees what it became. Only the site itself has to be asked again.
//!
//! Fuel bounds the re-firing however the rules are written. The rules that
//! give another site each make a call reading an older binding than the
//! call they rewrote, or reorder one operand to a fixed place, so none can
//! go round, but nothing here relies on that.

/// One rule: whether it applies in this walk at all, and the rewrite.
pub(super) struct Rule<C, E, S, T> {
    /// Whether the walk has this rule on.
    pub(super) on: fn(&C) -> bool,
    /// What the site is, where the rule's pattern and guards hold.
    pub(super) apply: fn(&mut C, &mut E, &S) -> Option<Step<S, T>>,
}

impl<C, E, S, T> Rule<C, E, S, T> {
    pub(super) const fn new(
        on: fn(&C) -> bool,
        apply: fn(&mut C, &mut E, &S) -> Option<Step<S, T>>,
    ) -> Self {
        Self { on, apply }
    }
}

/// What a rule made of a site.
pub(super) enum Step<S, T> {
    /// An answer no rule is asked about again.
    Done(T),
    /// Another site, which the rules are tried on again.
    Again(S),
}

/// The most rewrites one site goes through.
pub(super) const FUEL: usize = 8;

/// Try `rules` in order on `site`, then on each site a rewrite gives, until
/// one gives an answer, none fires, or the fuel is spent. A site no rule
/// answered comes back for the caller to write as it stands.
pub(super) fn rewrite<C, E, S, T>(
    context: &mut C,
    env: &mut E,
    rules: &[Rule<C, E, S, T>],
    mut site: S,
) -> Result<T, S> {
    for _ in 0..FUEL {
        let fired = rules.iter().find_map(|rule| {
            (rule.on)(context)
                .then(|| (rule.apply)(context, env, &site))
                .flatten()
        });
        match fired {
            None => return Err(site),
            Some(Step::Done(answer)) => return Ok(answer),
            Some(Step::Again(next)) => site = next,
        }
    }
    Err(site)
}
