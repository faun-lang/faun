//! Capture-free structural rebuilding, with optional bounded helper expansion.
//!
//! Structural normalization substitutes aliases without planning any inlining.
//! Expanding rewrites use the policy in `inline`; both preserve eager bindings
//! and protect recognized arithmetic definitions from changes to their equations.
//!
//! A walk may also evaluate: a construction or call whose every input is a
//! known value is written as the value it comes to, and a match on a known
//! sequence, which is not taken apart here, is evaluated whole. That is
//! constant folding. Such a walk reads each definition after what it calls,
//! publishing each as it goes, so a caller evaluates its callees as they now
//! stand; a refusal leaves the expression as it was.
//!
//! Every walk also decides what is already decided. A match on a local
//! written as a constructor, or as a known value, is the arm for that
//! constructor, with its fields what built it: nothing is tested. And a
//! match whose arms bind nothing and are the same asks a question whose
//! answer changes nothing, so it is that arm. Neither drops or reorders a
//! binding: the arm taken is the one that would have run, and the arms that
//! are not were never going to.
//!
//! A match whose answer is taken apart by the match after it, and by nothing
//! else, is one match: where every arm of the first ends by building a
//! constructor, the second's arm for that constructor is where it goes on.
//!
//! ```text
//! x = match s                     match s
//!   A => .. C(a)                    A => .. (arm C of the second, with a)
//!   B => .. D(b)          =>        B => .. (arm D of the second, with b)
//! match x
//!   C(p) => ..
//!   D(q) => ..
//! ```
//!
//! Nothing is built only to be asked what it is, and a field one
//! constructor carries and another does not is never held in one place for
//! both. Only where no arm of the second is taken by two arms of the first,
//! so nothing is copied twice.

mod semantic;

use rustc_hash::FxHashMap as HashMap;

use super::Operation::{Add, And, Div, Equals, LessThan, Mod, Monus, Mul, Or, Pow, Xor};
use super::budget::{Budget, FoldLimits};
use super::constant::Held;
use super::evaluate::{CannotFold, Evaluation, Evaluator};
use super::facts::Recognised;
use super::inline::{InlineLimits, Inlining};
use super::rules::{self, Rule, Step::Again, Step::Done};
use super::{
    Arm, Atom, Body, Expr, ExprId, FnId, Local, Operation, Program, Range, SequenceOperation,
};

/// What a walk does beyond rebuilding, substituting aliases and deciding
/// what is already decided, which every walk does.
#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent switches, one per rule a walk applies"
)]
struct Rules {
    /// Write a recognized operation an identity decides as what it
    /// decides: see [`Rewrite::identity`].
    identities: bool,
    /// Combine constants across recognized operations, and work a sum or
    /// product modulo a power of two out modulo it throughout.
    algebra: bool,
    /// Write a construction or call whose every input is known as the value
    /// it evaluates to, reading each definition after what it calls.
    evaluate: bool,
    /// Answer a call of an answered definition with what an earlier one
    /// with the same operands answered, where that is still in scope: see
    /// [`Rewrite::shared`].
    share: bool,
    /// Share too a construction, a match, or a call of a pure definition
    /// the same as one still in scope, so long as none of them builds a
    /// structure holding itself: see [`Rewrite::shape`]. Only once lists
    /// are fused, since a list two readers share is a list built.
    values: bool,
    /// Read nothing recognized: rewrite every definition, a recognizable
    /// one too, let no recognized operation decide anything, and leave
    /// questions of tags uncomposed for a walk that can first ask them as
    /// one comparison. What [`canonical`] does, so recognition reads what
    /// the walk wrote.
    blind: bool,
}

/// Rewrite every body as structural normalization does, before anything is
/// recognized, then forget what was.
///
/// Each body is rewritten alone -- no
/// call is expanded and nothing is evaluated -- so what a definition comes
/// to depends on its own body, never on budgets or the rest of the program,
/// and two ways of writing one equation that differ only in aliases, record
/// reads or decided matches are recognized alike.
pub(crate) fn canonical(program: &mut Program) {
    let rules = Rules {
        blind: true,
        ..Rules::default()
    };
    rewrite(program, None, rules, FoldLimits::default(), None);
    program.redefined();
}

/// Remove local aliases without expanding calls or dropping eager bindings.
pub(crate) fn structural(program: &mut Program) {
    rewrite(program, None, Rules::default(), FoldLimits::default(), None);
}

/// The same walk, writing a recognized operation an identity decides as
/// what it decides: see [`Rewrite::identity`].
#[cfg(test)]
pub(super) fn identities(program: &mut Program) {
    let rules = Rules {
        identities: true,
        ..Rules::default()
    };
    rewrite(program, None, rules, FoldLimits::default(), None);
}

/// Normalize aliases and inline nonrecursive helpers within the supplied limits.
/// A zero limit disables expansion without computing recursion or copy costs.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn normalize_with_limits(program: &mut Program, limits: InlineLimits) {
    let inlining = Inlining::new(program, limits);
    rewrite(
        program,
        inlining,
        Rules::default(),
        FoldLimits::default(),
        None,
    );
}

/// Inline as [`normalize_with_limits`] does, and apply the identities as
/// [`identities`] does, in one walk.
pub(super) fn inline_with_identities(program: &mut Program, limits: InlineLimits) {
    let inlining = Inlining::new(program, limits);
    let rules = Rules {
        identities: true,
        share: true,
        ..Rules::default()
    };
    rewrite(program, inlining, rules, FoldLimits::default(), None);
}

/// [`inline_with_identities`] for only `changed`: what a later stage wrote
/// calling small definitions it made for it.
pub(super) fn reinline(program: &mut Program, changed: &[FnId]) {
    if changed.is_empty() {
        return;
    }
    let inlining = Inlining::new(program, InlineLimits::default());
    let rules = Rules {
        identities: true,
        share: true,
        ..Rules::default()
    };
    rewrite(
        program,
        inlining,
        rules,
        FoldLimits::default(),
        Some(changed),
    );
}

/// Inline the definitions that keep a round of calls apart, and nothing
/// else: no identity, no folding. The stages after inlining change what
/// calls what -- simplifying a walk over a list it knows has one element
/// makes it call on rather than round -- and can leave two definitions
/// calling each other where neither did before, which only becomes a loop
/// below once one is copied into the other.
pub(super) fn join_rounds(program: &mut Program) {
    let inlining = Inlining::joining(program, InlineLimits::default());
    rewrite(
        program,
        inlining,
        Rules::default(),
        FoldLimits::default(),
        None,
    );
}

/// Constant folding alone: the walk evaluating what it can, without the
/// identities or algebra.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn fold(program: &mut Program) {
    fold_with_limits(program, FoldLimits::default());
}

/// [`fold`], within `limits`.
#[cfg(any(test, feature = "test-support"))]
pub(crate) fn fold_with_limits(program: &mut Program, limits: FoldLimits) {
    // Nothing to spend evaluates nothing, and the walk is not worth taking.
    if limits.work == 0 || limits.work_per_function == 0 {
        return;
    }
    let rules = Rules {
        evaluate: true,
        ..Rules::default()
    };
    rewrite(program, None, rules, limits, None);
}

/// Evaluate what is known, apply the identities, and combine what is left.
///
/// Each definition is rewritten after what it calls, so a call of one whose
/// answer is known outright is a constant by the time a caller asks, and
/// combining constants sees what evaluating made known in the same walk:
/// recognized addition and multiplication are associative and commutative,
/// so constants separated by one of them can be combined, and what a sum
/// modulo a power of two is worth knowing about cannot be seen until that
/// power is folded. Each definition draws at most a per-definition
/// allowance from one budget, so a costly answer declined in one leaves
/// the rest something to work with.
pub(super) fn simplify(program: &mut Program) {
    rewrite(program, None, SIMPLIFY, FoldLimits::default(), None);
}

/// [`simplify`] again for only `changed`, which a later stage handed
/// something new to read, with a budget of its own.
pub(super) fn resimplify(program: &mut Program, changed: &[FnId]) {
    if changed.is_empty() {
        return;
    }
    rewrite(
        program,
        None,
        SIMPLIFY,
        FoldLimits::default(),
        Some(changed),
    );
}

const SIMPLIFY: Rules = Rules {
    identities: true,
    algebra: true,
    evaluate: true,
    share: true,
    values: true,
    blind: false,
};

/// Whether every arm puts back the constructor it took apart, out of that
/// constructor's own fields and nothing else.
///
/// A match like that asks which one it is and answers with the one it is, so
/// it is the value it took apart. Nothing is dropped by saying so: an arm
/// shaped this way holds no bindings, so there is nothing in it to evaluate,
/// and validation has already said the arms cover the type. A type with no
/// constructors is left alone -- a match on one never answers, and the value
/// it took apart does.
///
/// `level` is the local the first arm binds its fields at.
fn rebuilt(program: &Program, arms: &[Arm], level: usize) -> bool {
    !arms.is_empty()
        && arms.iter().all(|arm| {
            arm.body.bindings.is_empty()
                && match program.expr(arm.body.tail) {
                    Expr::Con(ctor, fields) => {
                        ctor == arm.ctor
                            && program
                                .atoms(fields)
                                .iter()
                                .enumerate()
                                .all(|(at, held)| held.0.index() == level + at)
                    }
                    _ => false,
                }
        })
}

fn rewrite(
    program: &mut Program,
    inlining: Option<Inlining>,
    rules: Rules,
    limits: FoldLimits,
    only: Option<&[FnId]>,
) {
    let order = order(program, rules, only);
    let mut rewrite = Rewrite::new(program, inlining, rules, limits);
    let mut remaining = limits.work;
    // Inlining reads existing entries; writes only append. Keep all function
    // roots unchanged until the end, so later callers still inline original
    // bodies using the costs and depth limits computed for those bodies.
    let mut bodies = vec![];
    for id in order {
        if rewrite.recognised.answered(id) {
            continue;
        }
        let Some(body) = rewrite.program.function(id).body() else {
            continue;
        };
        if let Some(inlining) = &mut rewrite.inlining {
            inlining.begin_function();
        }
        let allowance = remaining.min(limits.work_per_function);
        rewrite.budget = Budget::new(FoldLimits {
            work: allowance,
            ..limits
        });
        let count = rewrite.program.params(id).len();
        let locals: Vec<_> = (0..count).map(|at| Atom(Local::at(at))).collect();
        rewrite.written.clear();
        rewrite.calls.clear();
        let body = rewrite.body(body, locals, count);
        remaining -= allowance - rewrite.budget.remaining;
        if rewrite.inlining.is_some() {
            bodies.push((id, body));
        } else {
            rewrite.program.replace_bodies(vec![(id, body)]);
        }
    }
    rewrite.program.replace_bodies(bodies);
}

/// The definitions a walk rewrites, `only` where it is given, in the order
/// it rewrites them. Evaluating reads callees as they now stand, so each is
/// rewritten, and published, before what calls it; a single definition
/// needs no order.
fn order(program: &Program, rules: Rules, only: Option<&[FnId]>) -> Vec<FnId> {
    match only {
        Some(&[id]) => vec![id],
        Some(ids) => super::calls::callees_first(program)
            .into_iter()
            .filter(|id| ids.contains(id))
            .collect(),
        None if rules.evaluate => super::calls::callees_first(program),
        None => (0..program.functions().len()).map(FnId::at).collect(),
    }
}

type CallRule<'a> = Rule<Rewrite<'a>, Scope, Called, Atom>;

/// What a call may be instead, asked in this order: evaluated, decided by
/// an identity, inlined, answered by an earlier call, then combined with
/// the constants around it.
fn call_rules<'a>() -> [CallRule<'a>; 10] {
    const fn always(_: &Rewrite<'_>) -> bool {
        true
    }
    const fn identities(rewrite: &Rewrite<'_>) -> bool {
        rewrite.rules.identities
    }
    const fn algebra(rewrite: &Rewrite<'_>) -> bool {
        rewrite.rules.algebra
    }
    [
        Rule::new(always, Rewrite::evaluated_call),
        Rule::new(always, Rewrite::counted),
        Rule::new(identities, Rewrite::float_double),
        Rule::new(identities, Rewrite::one_less),
        Rule::new(identities, Rewrite::identity),
        Rule::new(always, Rewrite::inlined),
        Rule::new(always, Rewrite::shared_call),
        Rule::new(algebra, Rewrite::chained),
        Rule::new(algebra, Rewrite::associated),
        Rule::new(algebra, Rewrite::reduced),
    ]
}

/// The most answers a question of tags is composed through.
const TAG_TREE_LEAVES: usize = 16;

/// Questions about tags, answered with tags.
enum Tags {
    Leaf(super::CtorId),
    /// The local asked about, where the answers are what was asked.
    Same(Atom),
    Split(Atom, Vec<(super::CtorId, Self)>),
}

/// What `map` turns `tag` into, where it turns it.
fn turned(map: &[(super::CtorId, super::CtorId)], tag: super::CtorId) -> Option<super::CtorId> {
    map.iter()
        .find(|&&(ctor, _)| ctor == tag)
        .map(|&(_, answer)| answer)
}

impl Tags {
    /// The same questions, of the locals `locals` gives for each level.
    fn renamed(&self, locals: &[Atom]) -> Self {
        match self {
            Self::Leaf(tag) => Self::Leaf(*tag),
            Self::Same(atom) => Self::Same(locals[atom.0.index()]),
            Self::Split(on, branches) => Self::Split(
                locals[on.0.index()],
                branches
                    .iter()
                    .map(|(ctor, tree)| (*ctor, tree.renamed(locals)))
                    .collect(),
            ),
        }
    }

    /// Each answer turned into another, or nothing where one is not turned.
    fn mapped(&self, to: &dyn Fn(super::CtorId) -> Option<super::CtorId>) -> Option<Self> {
        Some(match self {
            Self::Leaf(tag) => Self::Leaf(to(*tag)?),
            Self::Same(atom) => Self::Same(*atom),
            Self::Split(on, branches) => Self::Split(
                *on,
                branches
                    .iter()
                    .map(|(ctor, tree)| Some((*ctor, tree.mapped(to)?)))
                    .collect::<Option<_>>()?,
            ),
        })
    }

    /// Without questions whose answers agree, or that answer with the tag
    /// asked about.
    fn collapsed(self) -> Self {
        let Self::Split(on, branches) = self else {
            return self;
        };
        let branches: Vec<_> = branches
            .into_iter()
            .map(|(ctor, tree)| (ctor, tree.collapsed()))
            .collect();
        let leaf = |tree: &Self| match tree {
            Self::Leaf(tag) => Some(*tag),
            _ => None,
        };
        let tags: Option<Vec<_>> = branches.iter().map(|(_, tree)| leaf(tree)).collect();
        if let Some(tags) = tags {
            if tags.iter().all(|&tag| tag == tags[0]) {
                return Self::Leaf(tags[0]);
            }
            if branches
                .iter()
                .all(|&(ctor, ref tree)| leaf(tree) == Some(ctor))
            {
                return Self::Same(on);
            }
        }
        Self::Split(on, branches)
    }
}

struct Rewrite<'a> {
    program: &'a mut Program,
    /// Recognised definitions keep their bodies and are called, not
    /// expanded, so what they were recognised as stays true of them.
    recognised: Recognised,
    inlining: Option<Inlining>,
    rules: Rules,
    /// What evaluating and combining constants may still spend in the
    /// definition being rewritten.
    budget: Budget,
    inline_depth: usize,
    /// What wrote each local of the body being rebuilt, by level, where a
    /// binding of it did: a parameter or a field an arm took apart says
    /// nothing. Arms reuse levels, so this is cut back to where each begins.
    written: Vec<Option<ExprId>>,
    /// Each call of an answered definition written so far in the body being
    /// rebuilt, by what it was called with: the local it was written to, and
    /// the expression that wrote it, which says whether it still stands.
    calls: HashMap<Call, (Atom, ExprId)>,
    /// Indexed by definition, where the walk shares: see
    /// [`super::calls::pure`].
    pure: Vec<bool>,
    /// Each construction and match written so far, by its shape: see
    /// [`Rewrite::shape`].
    shapes: HashMap<Vec<usize>, (Atom, ExprId)>,
    /// What [`Self::known_as`] has written over, by level: a local an arm
    /// knows the constructor of is still the binding that wrote it.
    shadowed: Vec<(usize, ExprId)>,
}

/// The rest of a body still to write: the body, the binding it goes on
/// from, and what each binding before that came to.
struct Rest(Body, usize, Vec<Atom>);

/// A call site: what is called, and with what.
pub(super) struct Called(FnId, Vec<Atom>);

impl Called {
    fn parts(&self) -> (FnId, &[Atom]) {
        (self.0, &self.1)
    }
}

/// What a rule makes of a call.
type Step = rules::Step<Called, Atom>;

/// A call as [`Rewrite::shared`] compares it: the definition, and each
/// operand as the local it is or, where it is known, the value.
#[derive(Clone, PartialEq, Eq, Hash)]
struct Call(FnId, Vec<Operand>);

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Operand {
    Local(usize),
    Value(super::TypeId, Vec<u8>),
}
/// What rebuilding a match came to.
enum Matched {
    Rebuilt(Expr),
    /// Every arm binds nothing and says the same: this one.
    Same(Body),
    /// Every arm puts back what it took apart.
    Scrutinee,
}

struct Scope {
    bindings: Vec<ExprId>,
    level: usize,
    /// The level this scope began at, so that a local it bound can be
    /// looked back up: the one at `level` is `bindings[level - base]`.
    base: usize,
}

impl Scope {
    /// What wrote a local, where this scope wrote it. A parameter or a
    /// field an arm took apart was written somewhere else and says nothing.
    fn wrote(&self, atom: Atom) -> Option<ExprId> {
        let at = atom.0.index();
        self.bindings.get(at.checked_sub(self.base)?).copied()
    }
}
impl<'a> Rewrite<'a> {
    fn new(
        program: &'a mut Program,
        inlining: Option<Inlining>,
        rules: Rules,
        limits: FoldLimits,
    ) -> Self {
        let recognised = if rules.blind {
            Recognised::none(program.functions().len())
        } else {
            program.recognised().clone()
        };
        let pure = if rules.values {
            super::calls::pure(program, &recognised)
        } else {
            Vec::new()
        };
        Self {
            pure,
            program,
            recognised,
            inlining,
            rules,
            budget: Budget::new(limits),
            inline_depth: 0,
            written: Vec::new(),
            calls: HashMap::default(),
            shapes: HashMap::default(),
            shadowed: Vec::new(),
        }
    }
}

impl Rewrite<'_> {
    fn body(&mut self, body: Body, locals: Vec<Atom>, level: usize) -> Body {
        self.body_with(level, |rewrite, scope| rewrite.into(body, locals, scope))
    }

    /// A body of its own, whose first binding takes `level`, written by
    /// `write`.
    fn body_with(
        &mut self,
        level: usize,
        write: impl FnOnce(&mut Self, &mut Scope) -> Atom,
    ) -> Body {
        self.written.truncate(level);
        self.written.resize(level, None);
        let mut scope = Scope {
            bindings: vec![],
            level,
            base: level,
        };
        let answer = write(self, &mut scope);
        let tail = if answer.0.index() + 1 == scope.level && !scope.bindings.is_empty() {
            scope.bindings.pop().expect("the last binding")
        } else if let Some(value) = self.known(answer).filter(|_| self.rules.evaluate) {
            // Answering the value itself leaves the local it was written to
            // for nothing to read, where it was written before a match
            // whose other arms do not need it.
            self.program.terms.push(Expr::Static(value))
        } else {
            self.program.terms.push(Expr::Atom(answer))
        };
        self.program.terms.hold_body(&scope.bindings, tail)
    }
    fn into(&mut self, body: Body, locals: Vec<Atom>, scope: &mut Scope) -> Atom {
        let from = body.bindings.range().start;
        self.rest_of(body, from, locals, scope, Vec::new())
    }

    /// The rest of `body`, from its binding at `from`, where `locals` has
    /// what each binding before it came to, then the rest of each body
    /// `then` holds, the last first, each taking what the one before came
    /// to as its next binding.
    fn rest_of(
        &mut self,
        body: Body,
        from: usize,
        mut locals: Vec<Atom>,
        scope: &mut Scope,
        mut then: Vec<Rest>,
    ) -> Atom {
        let first = locals.len() - (from - body.bindings.range().start);
        let cased = self.cased(body, first);
        let last = body.bindings.range().end;
        for at in from..last {
            if let Some((scrutinee, first, second)) = cased
                && at + 1 == last
                && !self.decided(locals[scrutinee.0.index()])
            {
                let value = self.merged(scrutinee, first, second, &locals, scope);
                return self.then(value, scope, then);
            }
            let binding = self.program.terms.binding(at);
            if let Some(arm) = self.floats(binding, &locals) {
                then.push(Rest(body, at + 1, locals.clone()));
                return self.floated(binding, arm, &locals, then, scope);
            }
            let value = self.expr(binding, &locals, scope);
            locals.push(value);
        }
        // A body ending in a match of one arm, itself floated, floats it
        // for the rest it is followed by.
        if !then.is_empty()
            && let Some(arm) = self.floats(body.tail, &locals)
        {
            return self.floated(body.tail, arm, &locals, then, scope);
        }
        let value = self.expr(body.tail, &locals, scope);
        self.then(value, scope, then)
    }

    /// What a body came to, `value`, taken by the rest `then` holds last.
    fn then(&mut self, value: Atom, scope: &mut Scope, mut then: Vec<Rest>) -> Atom {
        let Some(Rest(body, from, mut locals)) = then.pop() else {
            return value;
        };
        locals.push(value);
        self.rest_of(body, from, locals, scope, then)
    }

    /// The one arm of a match a binding is, where it takes apart what no
    /// match has yet: see [`Self::floated`].
    fn floats(&self, binding: ExprId, locals: &[Atom]) -> Option<Arm> {
        let Expr::Match(value, arms) = self.program.expr(binding) else {
            return None;
        };
        let &[arm] = self.program.arms(arms) else {
            return None;
        };
        (!self.decided(locals[value.0.index()])).then_some(arm)
    }

    /// Whether a local was written as a constructor or a known value, so
    /// that a match on it takes one arm: see [`Self::selected`].
    fn decided(&self, atom: Atom) -> bool {
        self.written
            .get(atom.0.index())
            .copied()
            .flatten()
            .is_some_and(|wrote| {
                matches!(self.program.expr(wrote), Expr::Con(..) | Expr::Static(..))
            })
    }

    /// A match of one arm, which cannot but take it, taking apart a value
    /// the rest of the body may take apart again: the rest goes on in that
    /// arm, where what it took apart is known, and every match on it again
    /// takes the same arm rather than asking. The rest is the rest of every
    /// body this one's arm is floated in too, `then`, so what an arm worked
    /// out stays in scope for all that follows it, and a later call the
    /// same as one in it answers what that one did.
    ///
    /// ```text
    /// x = match s                  match s
    ///   U8(n) => .. a                U8(n) =>
    /// y = match s          =>          .. a      (x)
    ///   U8(m) => .. b                  .. b      (y, with m as n)
    /// ```
    fn floated(
        &mut self,
        binding: ExprId,
        arm: Arm,
        locals: &[Atom],
        then: Vec<Rest>,
        scope: &mut Scope,
    ) -> Atom {
        let Expr::Match(value, _) = self.program.expr(binding) else {
            unreachable!("a floated binding is a match");
        };
        let scrutinee = locals[value.0.index()];
        let fields = self.program.fields(arm.ctor).len();
        let taken: Vec<_> = (0..fields)
            .map(|at| Atom(Local::at(scope.level + at)))
            .collect();
        self.written.truncate(scope.level);
        let rest = self.known_as(scrutinee, arm.ctor, &taken, |rewrite| {
            rewrite.body_with(scope.level + fields, |rewrite, arm_scope| {
                let mut inner = locals.to_vec();
                inner.extend(&taken);
                let from = arm.body.bindings.range().start;
                rewrite.rest_of(arm.body, from, inner, arm_scope, then)
            })
        });
        self.written.truncate(scope.level);
        let arms = self.program.terms.hold_arms(&[Arm {
            ctor: arm.ctor,
            body: rest,
        }]);
        self.push(Expr::Match(scrutinee, arms), scope)
    }

    /// Where a body ends by taking apart, and only taking apart, a match
    /// that is its last binding, each of whose arms ends by building a
    /// constructor, no two of them one taken by the same arm: the first
    /// match's scrutinee and arms, and the second's arms. `level` is the
    /// level of the body's first binding.
    fn cased(&self, body: Body, level: usize) -> Option<(Atom, Range, Range)> {
        let bindings = self.program.bindings(body.bindings);
        let &last = bindings.last()?;
        let answer = level + bindings.len() - 1;
        let Expr::Match(Atom(on), second) = self.program.expr(body.tail) else {
            return None;
        };
        let Expr::Match(scrutinee, first) = self.program.expr(last) else {
            return None;
        };
        if on.index() != answer {
            return None;
        }
        let mut taken = vec![false; self.program.arms(second).len()];
        for arm in self.program.arms(first) {
            let Expr::Con(ctor, _) = self.program.expr(arm.body.tail) else {
                return None;
            };
            let at = self
                .program
                .arms(second)
                .iter()
                .position(|arm| arm.ctor == ctor)?;
            if std::mem::replace(&mut taken[at], true) {
                return None;
            }
        }
        let read = self.program.arms(second).iter().any(|arm| {
            super::walk::reads(
                self.program,
                super::walk::within(self.program, arm.body),
                answer,
            ) > 0
        });
        (!read).then_some((scrutinee, first, second))
    }

    /// The first match, each arm going on into the second's arm for the
    /// constructor it built.
    fn merged(
        &mut self,
        scrutinee: Atom,
        first: Range,
        second: Range,
        locals: &[Atom],
        scope: &mut Scope,
    ) -> Atom {
        let second: Vec<_> = self.program.arms(second).to_vec();
        let mut found = vec![];
        for at in first.range() {
            let arm = self.program.terms.arm(at);
            let fields = self.program.fields(arm.ctor).len();
            let mut inner = locals.to_vec();
            inner.extend((0..fields).map(|at| Atom(Local::at(scope.level + at))));
            self.written.truncate(scope.level);
            let body = self.body_with(scope.level + fields, |rewrite, arm_scope| {
                for at in arm.body.bindings.range() {
                    let binding = rewrite.program.terms.binding(at);
                    let value = rewrite.expr(binding, &inner, arm_scope);
                    inner.push(value);
                }
                let Expr::Con(ctor, built) = rewrite.program.expr(arm.body.tail) else {
                    unreachable!("every arm was seen to build a constructor");
                };
                let built = rewrite.args(built, &inner);
                let next = second
                    .iter()
                    .find(|arm| arm.ctor == ctor)
                    .expect("every constructor built has an arm");
                // Nothing reads the first match's answer, so its level may
                // stand for anything.
                let mut going = locals.to_vec();
                going.push(locals[scrutinee.0.index()]);
                going.extend(built);
                rewrite.into(next.body, going, arm_scope)
            });
            found.push(Arm {
                ctor: arm.ctor,
                body,
            });
        }
        self.written.truncate(scope.level);
        let arms = self.program.terms.hold_arms(&found);
        self.push(Expr::Match(locals[scrutinee.0.index()], arms), scope)
    }
    fn expr(&mut self, id: ExprId, locals: &[Atom], scope: &mut Scope) -> Atom {
        let expr = match self.program.expr(id) {
            Expr::Atom(a) => return locals[a.0.index()],
            value @ Expr::Static(..) => value,
            Expr::Con(ctor, fields) => {
                let fields = self.args(fields, locals);
                if let Some(held) = self.evaluated(&fields, scope, |evaluator, fields, budget| {
                    evaluator.construct(ctor, &fields, budget)
                }) {
                    return held;
                }
                Expr::Con(ctor, self.program.terms.hold_atoms(fields.into_iter()))
            }
            Expr::Call(called, args) => {
                let args = self.args(args, locals);
                return self.call(Called(called, args), scope);
            }
            Expr::Match(value, arms) => {
                let scrutinee = locals[value.0.index()];
                if let Some(answer) = self.selected(scrutinee, arms, locals, scope) {
                    return answer;
                }
                if let Some(answer) = self.evaluated_match(id, scrutinee, locals, scope) {
                    return answer;
                }
                // Composing questions of tags is left to a walk reading what
                // is recognized, which may first ask them as one comparison.
                if !self.rules.blind {
                    if let Some(answer) = self.composed(scrutinee, arms, scope) {
                        return answer;
                    }
                    if let Some(answer) = self.composed_within(scrutinee, arms, scope) {
                        return answer;
                    }
                }
                match self.matching(value, arms, locals, scope) {
                    Matched::Rebuilt(held) => held,
                    Matched::Same(body) => return self.spliced(body, scope),
                    // One that puts back what it took apart is what it took apart.
                    Matched::Scrutinee => return scrutinee,
                }
            }
        };
        let Some(shape) = self.shape(expr, scope.level) else {
            return self.push(expr, scope);
        };
        if let Some(&(held, wrote)) = self.shapes.get(&shape)
            && self.stands(held, wrote, scope)
        {
            return held;
        }
        let held = self.push(expr, scope);
        let wrote = *scope.bindings.last().expect("a value just written");
        self.shapes.insert(shape, (held, wrote));
        held
    }

    /// A construction or a match as two the same are written, where the walk
    /// shares and every call in it is pure: each local read by its level
    /// where it was bound before `base`, and by how far past `base` where
    /// the match bound it itself, so that two matches the same but for the
    /// levels their arms bind are one shape.
    fn shape(&self, expr: Expr, base: usize) -> Option<Vec<usize>> {
        if !self.rules.values || matches!(expr, Expr::Atom(_) | Expr::Static(_)) {
            return None;
        }
        let mut shape = Vec::new();
        self.shaped(expr, base, &mut shape)?;
        Some(shape)
    }

    fn shaped(&self, expr: Expr, base: usize, shape: &mut Vec<usize>) -> Option<()> {
        let local = |atom: Atom| {
            let level = atom.0.index();
            level
                .checked_sub(base)
                .map_or(level * 2, |past| past * 2 + 1)
        };
        let atoms = |shape: &mut Vec<usize>, range| {
            let atoms = self.program.atoms(range);
            shape.push(atoms.len());
            shape.extend(atoms.iter().map(|&atom| local(atom)));
        };
        match expr {
            Expr::Atom(atom) => shape.extend([0, local(atom)]),
            Expr::Static(value) => {
                let constant = self.program.constant(value);
                let bytes = constant.encoded();
                shape.extend([1, constant.owner().index(), bytes.len()]);
                shape.extend(bytes.iter().map(|&byte| usize::from(byte)));
            }
            Expr::Con(ctor, fields) => {
                (!self.holds_itself(self.program.ctor(ctor).owner)).then_some(())?;
                shape.extend([2, ctor.index()]);
                atoms(shape, fields);
            }
            Expr::Call(called, args) => {
                (self.recognised.answered(called) || self.shareable(called)).then_some(())?;
                shape.extend([3, called.index()]);
                atoms(shape, args);
            }
            Expr::Match(on, arms) => {
                let arms = self.program.arms(arms);
                shape.extend([4, local(on), arms.len()]);
                for arm in arms {
                    shape.push(arm.ctor.index());
                    self.shaped_body(arm.body, base, shape)?;
                }
            }
        }
        Some(())
    }

    fn shaped_body(&self, body: Body, base: usize, shape: &mut Vec<usize>) -> Option<()> {
        let bindings = self.program.bindings(body.bindings);
        shape.push(bindings.len());
        for &binding in bindings {
            self.shaped(self.program.expr(binding), base, shape)?;
        }
        self.shaped(self.program.expr(body.tail), base, shape)
    }

    /// A call answered by an earlier one the same, still in scope.
    fn shared_call(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        self.shared(&self.sharing(called, args)?, scope).map(Done)
    }

    /// A call whose every input is known, as the value it comes to.
    fn evaluated_call(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        self.evaluated(args, scope, |evaluator, args, budget| {
            evaluator.call(called, args, budget)
        })
        .map(Done)
    }

    /// A call of a definition the walk inlines, as its body.
    fn inlined(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        let body = self.callee(called)?;
        self.inline_depth += 1;
        let answer = self.into(body, args.to_vec(), scope);
        self.inline_depth -= 1;
        Some(Done(answer))
    }

    /// A call, as the first of [`call_rules`] to fire rewrites it, the rules
    /// asked again of each call a rule rewrites it to, or else written as
    /// it stands.
    fn call(&mut self, site: Called, scope: &mut Scope) -> Atom {
        let Called(called, args) = match rules::rewrite(self, scope, &call_rules(), site) {
            Ok(answer) => return answer,
            Err(site) => site,
        };
        let call = self.sharing(called, &args);
        let args = self.held(&args);
        let held = self.push(Expr::Call(called, args), scope);
        if let Some(call) = call {
            let wrote = *scope.bindings.last().expect("a call just written");
            self.calls.insert(call, (held, wrote));
        }
        held
    }

    /// The arm a match takes where its scrutinee was written as one
    /// constructor or as a known value, taken where the match stands: its
    /// fields are the atoms that built it, or known values of their own.
    fn selected(
        &mut self,
        scrutinee: Atom,
        arms: Range,
        locals: &[Atom],
        scope: &mut Scope,
    ) -> Option<Atom> {
        let wrote = (*self.written.get(scrutinee.0.index())?)?;
        let (ctor, fields) = match self.program.expr(wrote) {
            Expr::Con(ctor, fields) => (ctor, self.program.atoms(fields).to_vec()),
            Expr::Static(value) => self.opened(value, scope)?,
            Expr::Atom(_) | Expr::Call(..) | Expr::Match(..) => return None,
        };
        let arm = self
            .program
            .arms(arms)
            .iter()
            .find(|arm| arm.ctor == ctor)
            .copied()?;
        let mut inner = locals.to_vec();
        inner.extend(fields);
        Some(self.into(arm.body, inner, scope))
    }

    /// A match turning tags into tags, on a local another match wrote out of
    /// tags alone, is one match: the first's questions, each answered with
    /// the tag the second turns its answer into. So `a < b`, which asks an
    /// order and turns it into a truth, asks the comparison itself, and
    /// what a branch on it says of `a` and `b` is seen by range analysis.
    /// Nothing is copied but tags, and a question whose answers agree, or
    /// that answers with what it asked about, is not asked at all.
    fn composed(&mut self, scrutinee: Atom, arms: Range, scope: &mut Scope) -> Option<Atom> {
        let wrote = (*self.written.get(scrutinee.0.index())?)?;
        let inner = self.tag_tree(wrote, TAG_TREE_LEAVES)?;
        let outer = self.tag_map(arms)?;
        let mapped = inner.mapped(&|tag| turned(&outer, tag))?;
        Some(self.emit_tags(mapped.collapsed(), scope))
    }

    /// The same, where the other match works something out in an arm
    /// before it answers with tags, as taking a number apart to compare it
    /// does: that match again, each arm working out what it did and
    /// answering with the tags this one turns its answer into. The first
    /// match is left for cleanup, nothing reading it any more.
    fn composed_within(&mut self, scrutinee: Atom, arms: Range, scope: &mut Scope) -> Option<Atom> {
        let level = scrutinee.0.index();
        let wrote = (*self.written.get(level)?)?;
        let Expr::Match(on, inner) = self.program.expr(wrote) else {
            return None;
        };
        let outer = self.tag_map(arms)?;
        let inner = self.program.arms(inner).to_vec();
        let leaves = TAG_TREE_LEAVES / inner.len().max(1);
        let trees = inner
            .iter()
            .map(|arm| {
                let tree = self.tag_tree(arm.body.tail, leaves)?;
                tree.mapped(&|tag| turned(&outer, tag))
            })
            .collect::<Option<Vec<_>>>()?;
        let mut found = vec![];
        for (arm, tree) in inner.iter().zip(trees) {
            let fields = self.program.fields(arm.ctor).len();
            // What the arm read from outside it is where it was, and it
            // began at the level its match was written at.
            let mut locals: Vec<_> = (0..level).map(|at| Atom(Local::at(at))).collect();
            locals.extend((0..fields).map(|at| Atom(Local::at(scope.level + at))));
            self.written.truncate(scope.level);
            let body = self.body_with(scope.level + fields, |rewrite, arm_scope| {
                for at in arm.body.bindings.range() {
                    let binding = rewrite.program.terms.binding(at);
                    let value = rewrite.expr(binding, &locals, arm_scope);
                    locals.push(value);
                }
                rewrite.emit_tags(tree.renamed(&locals).collapsed(), arm_scope)
            });
            found.push(Arm {
                ctor: arm.ctor,
                body,
            });
        }
        self.written.truncate(scope.level);
        let arms = self.program.terms.hold_arms(&found);
        Some(self.push(Expr::Match(on, arms), scope))
    }

    /// Whether `a` is below `b`, and if not whether it is `b`, is three
    /// answers, one each for below, equal and above. Where two of them
    /// agree it is one question: below or equal is not above, and above or
    /// below is not equal.
    fn ordered(&mut self, tags: Tags, scope: &mut Scope) -> Tags {
        let Tags::Split(on, ref branches) = tags else {
            return tags;
        };
        let Some((less, a, b)) = self.asked(on) else {
            return tags;
        };
        let LessThan {
            yes: lt_yes,
            no: lt_no,
        } = less
        else {
            return tags;
        };
        let branch = |ctor| {
            branches
                .iter()
                .find(|&&(tag, _)| tag == ctor)
                .map(|(_, tree)| tree)
        };
        let (Some(Tags::Leaf(below)), Some(rest)) = (branch(lt_yes), branch(lt_no)) else {
            return tags;
        };
        let Some((asked, equal, above)) = self.equality(rest, a, b) else {
            return tags;
        };
        let below = *below;
        if below == above {
            return Tags::Same(asked);
        }
        if below == equal {
            let called = self.called(on).expect("asked is a call");
            let args = self.program.terms.hold_atoms([b, a].into_iter());
            let swapped = self.push(Expr::Call(called, args), scope);
            // The arms in the order the type declares its tags.
            let branches = branches
                .iter()
                .map(|&(tag, _)| (tag, Tags::Leaf(if tag == lt_yes { above } else { below })))
                .collect();
            return Tags::Split(swapped, branches);
        }
        tags
    }

    /// Where `tree` answers whether `a` equals `b`: what asks it, and its
    /// answers for equal and not.
    fn equality(
        &self,
        tree: &Tags,
        a: Atom,
        b: Atom,
    ) -> Option<(Atom, super::CtorId, super::CtorId)> {
        let &Tags::Same(asked) = tree else {
            return None;
        };
        let (Equals { yes, no }, x, y) = self.asked(asked)? else {
            return None;
        };
        ((x, y) == (a, b) || (x, y) == (b, a)).then_some((asked, yes, no))
    }

    /// The operation a local was written as a call of, with its two
    /// operands.
    fn asked(&self, atom: Atom) -> Option<(Operation, Atom, Atom)> {
        let wrote = (*self.written.get(atom.0.index())?)?;
        let Expr::Call(called, args) = self.program.expr(wrote) else {
            return None;
        };
        let operation = self.recognised.operations[called.index()]?;
        let &[a, b] = self.program.atoms(args) else {
            return None;
        };
        Some((operation, a, b))
    }

    fn called(&self, atom: Atom) -> Option<FnId> {
        let wrote = (*self.written.get(atom.0.index())?)?;
        match self.program.expr(wrote) {
            Expr::Call(called, _) => Some(called),
            _ => None,
        }
    }

    /// The tag each arm of a match answers with, by the tag it takes, where
    /// every arm only answers with one.
    fn tag_map(&self, arms: Range) -> Option<Vec<(super::CtorId, super::CtorId)>> {
        self.program
            .arms(arms)
            .iter()
            .map(|arm| {
                let tag = arm
                    .body
                    .bindings
                    .is_empty()
                    .then(|| self.tag(arm.body.tail))??;
                Some((arm.ctor, tag))
            })
            .collect()
    }

    /// The nullary constructor `expr` builds or knows, where it is one.
    fn tag(&self, expr: ExprId) -> Option<super::CtorId> {
        let ctor = match self.program.expr(expr) {
            Expr::Con(ctor, fields) if fields.is_empty() => ctor,
            Expr::Static(value) => self.program.constant(value).constructor(),
            _ => return None,
        };
        self.program.fields(ctor).is_empty().then_some(ctor)
    }

    /// `expr` as questions about tags answered with tags, of at most
    /// `leaves` answers.
    fn tag_tree(&self, expr: ExprId, leaves: usize) -> Option<Tags> {
        if let Some(tag) = self.tag(expr) {
            return Some(Tags::Leaf(tag));
        }
        let Expr::Match(on, arms) = self.program.expr(expr) else {
            return None;
        };
        let arms = self.program.arms(arms);
        if arms.len() > leaves {
            return None;
        }
        let branches = arms
            .iter()
            .map(|arm| {
                let bare = arm.body.bindings.is_empty() && self.program.fields(arm.ctor).is_empty();
                let tree = bare.then(|| self.tag_tree(arm.body.tail, leaves / arms.len()))??;
                Some((arm.ctor, tree))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Tags::Split(on, branches))
    }

    fn emit_tags(&mut self, tags: Tags, scope: &mut Scope) -> Atom {
        let tags = self.ordered(tags, scope);
        match tags {
            Tags::Leaf(tag) => self.push(Expr::Con(tag, Range::default()), scope),
            Tags::Same(atom) => atom,
            Tags::Split(on, branches) => {
                let arms: Vec<_> = branches
                    .into_iter()
                    .map(|(ctor, tree)| Arm {
                        ctor,
                        body: self.body_with(scope.level, |rewrite, inner| {
                            rewrite.emit_tags(tree, inner)
                        }),
                    })
                    .collect();
                self.written.truncate(scope.level);
                let arms = self.program.terms.hold_arms(&arms);
                self.push(Expr::Match(on, arms), scope)
            }
        }
    }

    /// The length of a sequence written as a known value, which is a number
    /// known too.
    fn counted(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        let (Some(SequenceOperation::Length), &[sequence]) =
            (self.recognised.sequences[called.index()], args)
        else {
            return None;
        };
        let Expr::Static(value) = self.written_as(sequence)? else {
            return None;
        };
        let value = super::single_field_constant(self.program, self.program.constant(value));
        let super::ConstantView::Spine(spine) = value.view() else {
            return None;
        };
        let length = u64::try_from(spine.length()).ok()?;
        self.number_of(called, length, scope).map(Done)
    }

    /// A known value's constructor, and each of its fields as a known value
    /// of its own, bound here. A known number is its successor's step, or
    /// nought; a known sequence is left to be taken apart where it runs.
    fn opened(
        &mut self,
        value: super::ConstId,
        scope: &mut Scope,
    ) -> Option<(super::CtorId, Vec<Atom>)> {
        let constant = self.program.constant(value);
        let ctor = constant.constructor();
        let owner = constant.owner();
        let fields: Vec<_> = match constant.view() {
            super::ConstantView::Constructor(view) => {
                let mut budget = Budget::new(FoldLimits::default());
                let fields = view.fields_within(&mut budget).ok()?;
                fields
                    .into_iter()
                    .map(|field| field.to_owned(&mut budget))
                    .collect::<Result<Vec<_>, _>>()
                    .ok()?
                    .into_iter()
                    .map(|held| self.program.intern_held(&held))
                    .collect()
            }
            super::ConstantView::Natural(number) => {
                let number = number.to_biguint();
                if number == num_bigint::BigUint::default() {
                    Vec::new()
                } else {
                    let before = number - 1u8;
                    vec![self.program.intern_natural(owner, &before).ok()?]
                }
            }
            super::ConstantView::Spine(_) => return None,
        };
        let atoms = fields
            .into_iter()
            .map(|field| self.push(Expr::Static(field), scope))
            .collect();
        Some((ctor, atoms))
    }

    /// A rebuilt arm that binds nothing, laid where the match stood: its
    /// bindings already take the levels they would take here.
    fn spliced(&mut self, body: Body, scope: &mut Scope) -> Atom {
        for &binding in &self.program.bindings(body.bindings).to_vec() {
            scope.bindings.push(binding);
            self.written.truncate(scope.level);
            self.written.push(Some(binding));
            scope.level += 1;
        }
        match self.program.expr(body.tail) {
            Expr::Atom(atom) => atom,
            tail => self.push(tail, scope),
        }
    }
    /// `write`, where `atom` is known to be `ctor` of `fields`: in an arm,
    /// what its match took apart is what the arm took it apart into, and a
    /// match on it again takes that arm.
    fn known_as<T>(
        &mut self,
        atom: Atom,
        ctor: super::CtorId,
        fields: &[Atom],
        write: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let level = atom.0.index();
        let Some(&before) = self.written.get(level) else {
            return write(self);
        };
        let fields = self.program.terms.hold_atoms(fields.iter().copied());
        let known = self.program.terms.push(Expr::Con(ctor, fields));
        self.written[level] = Some(known);
        if let Some(before) = before {
            self.shadowed.push((level, before));
        }
        let written = write(self);
        if before.is_some() {
            self.shadowed.pop();
        }
        if let Some(entry) = self.written.get_mut(level) {
            *entry = before;
        }
        written
    }

    /// Rebuild a match's arms: as a match, as the one arm every arm is, or
    /// as its scrutinee where every arm puts back the constructor it took
    /// apart.
    fn matching(&mut self, value: Atom, arms: Range, locals: &[Atom], scope: &Scope) -> Matched {
        let mut found = vec![];
        let scrutinee = locals[value.0.index()];
        for at in arms.range() {
            let arm = self.program.terms.arm(at);
            let fields = self.program.fields(arm.ctor).len();
            let taken: Vec<_> = (0..fields)
                .map(|at| Atom(Local::at(scope.level + at)))
                .collect();
            let mut inner = locals.to_vec();
            inner.extend(&taken);
            // A sibling arm's bindings may have taken this arm's fields'
            // levels, and what wrote them is not what this arm binds.
            self.written.truncate(scope.level);
            let body = self.known_as(scrutinee, arm.ctor, &taken, |rewrite| {
                rewrite.body(arm.body, inner, scope.level + fields)
            });
            found.push(Arm {
                ctor: arm.ctor,
                body,
            });
        }
        self.written.truncate(scope.level);
        if rebuilt(self.program, &found, scope.level) {
            return Matched::Scrutinee;
        }
        if let Some((first, rest)) = found.split_first()
            && self.program.fields(first.ctor).is_empty()
            && rest.iter().all(|arm| {
                self.program.fields(arm.ctor).is_empty() && self.same_body(first.body, arm.body)
            })
        {
            return Matched::Same(first.body);
        }
        let arms = self.program.terms.hold_arms(&found);
        Matched::Rebuilt(Expr::Match(locals[value.0.index()], arms))
    }

    /// Whether two bodies say the same thing, term for term. Known values
    /// are compared by what they are, since equal ones are held apart.
    fn same_body(&self, a: Body, b: Body) -> bool {
        let (left, right) = (
            self.program.bindings(a.bindings),
            self.program.bindings(b.bindings),
        );
        left.len() == right.len()
            && left.iter().zip(right).all(|(&x, &y)| self.same_expr(x, y))
            && self.same_expr(a.tail, b.tail)
    }

    fn same_expr(&self, a: ExprId, b: ExprId) -> bool {
        match (self.program.expr(a), self.program.expr(b)) {
            (Expr::Atom(x), Expr::Atom(y)) => x == y,
            (Expr::Static(x), Expr::Static(y)) => {
                let (x, y) = (self.program.constant(x), self.program.constant(y));
                x.owner() == y.owner() && x.encoded() == y.encoded()
            }
            (Expr::Con(x, xs), Expr::Con(y, ys)) => {
                x == y && self.program.atoms(xs) == self.program.atoms(ys)
            }
            (Expr::Call(x, xs), Expr::Call(y, ys)) => {
                x == y && self.program.atoms(xs) == self.program.atoms(ys)
            }
            (Expr::Match(x, xs), Expr::Match(y, ys)) => {
                let (xs, ys) = (self.program.arms(xs), self.program.arms(ys));
                x == y
                    && xs.len() == ys.len()
                    && xs
                        .iter()
                        .zip(ys)
                        .all(|(l, r)| l.ctor == r.ctor && self.same_body(l.body, r.body))
            }
            _ => false,
        }
    }

    /// A recognized operation whose answer an identity decides, from which
    /// operands are the same local or known to be nought or one: that
    /// operand, nought, or the comparison's answer. Its argument bindings
    /// stay where they are, so nothing they evaluate is dropped.
    fn identity(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        if let Some(op) = self.recognised.sequences[called.index()] {
            return self.sequence_identity(op, called, args, scope).map(Done);
        }
        let (op, &[a, b]) = (self.op(called)?, args) else {
            return None;
        };
        let (x, y) = (self.small(a), self.small(b));
        Some(Done(match op {
            Add | Or | Xor if x == Some(0) => b,
            Add | Monus | Mod | Or | Xor if y == Some(0) => a,
            Mul if x == Some(1) => b,
            Mul | Div | Pow if y == Some(1) => a,
            And | Or if a == b => a,
            Monus if x == Some(0) => self.number_of(called, 0, scope)?,
            Monus | Xor if a == b => self.number_of(called, 0, scope)?,
            Mul | Div | And if x == Some(0) || y == Some(0) => self.number_of(called, 0, scope)?,
            Mod if x == Some(0) || y == Some(1) || a == b => self.number_of(called, 0, scope)?,
            Equals { yes, .. } if a == b => self.nullary(yes, scope),
            LessThan { no, .. } if a == b => self.nullary(no, scope),
            // In particular x/x is NOT one: x can be zero.
            _ => return None,
        }))
    }

    /// A recognized sequence operation an empty operand decides, or a count
    /// of nought: the operand it answers, the empty one it took, or nought
    /// for the length of an empty sequence.
    fn sequence_identity(
        &mut self,
        op: SequenceOperation,
        called: FnId,
        args: &[Atom],
        scope: &mut Scope,
    ) -> Option<Atom> {
        use SequenceOperation::{Drop, Length, Prepend, Replace, ReverseOnto, Take};
        let params = self.program.params(called).to_vec();
        let empty = |at: usize| self.empty(args[at], params[at]);
        Some(match op {
            ReverseOnto | Prepend if empty(0) => args[1],
            Prepend if empty(1) => args[0],
            ReverseOnto => return self.short_reverse(called, args, scope),
            Drop if self.small(args[1]) == Some(0) => args[0],
            Drop | Take | Replace if empty(0) => args[0],
            Length if empty(0) => self.number_of(called, 0, scope)?,
            _ => return None,
        })
    }

    /// Whether a local of sequence type `ty` is known to be the empty one:
    /// written as its empty constructor or as a known value built by it, or
    /// taken apart into it by the arm it is read in.
    fn empty(&self, atom: Atom, ty: super::TypeId) -> bool {
        let Some((nil, _, _)) = super::sequence::sequence(self.program, ty) else {
            return false;
        };
        match self.written_as(atom) {
            Some(Expr::Con(ctor, _)) => ctor == nil,
            Some(Expr::Static(value)) => self.program.constant(value).constructor() == nil,
            _ => false,
        }
    }

    /// One taken from a natural known to be a step, where the step is of
    /// the type `called` answers: what an arm taking the natural apart into
    /// its step took, nothing worked out, as where the arm's field is read
    /// instead.
    fn one_less(&mut self, _: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        let (Monus, &[atom, by]) = (self.op(called)?, args) else {
            return None;
        };
        let Expr::Con(ctor, fields) = self
            .written_as(atom)
            .filter(|_| self.small(by) == Some(1))?
        else {
            return None;
        };
        let owner = self.program.ctor(ctor).owner;
        let (_, step) = super::arithmetic::natural(self.program, owner)?;
        (ctor == step && self.program.function(called).result == owner)
            .then(|| Done(self.program.atoms(fields)[0]))
    }

    /// Whether a local is a natural known to be nought or one: written as
    /// that number, or as nought or the step of nought.
    fn small(&self, atom: Atom) -> Option<u8> {
        match self.written_as(atom)? {
            Expr::Static(value) => {
                let super::ConstantView::Natural(number) = self.program.constant(value).view()
                else {
                    return None;
                };
                u8::try_from(number.try_u64().ok()?)
                    .ok()
                    .filter(|&n| n <= 1)
            }
            Expr::Con(ctor, fields) => {
                let owner = self.program.ctor(ctor).owner;
                let (nil, _) = super::arithmetic::natural(self.program, owner)?;
                if ctor == nil {
                    Some(0)
                } else {
                    (self.small(self.program.atoms(fields)[0]) == Some(0)).then_some(1)
                }
            }
            Expr::Atom(_) | Expr::Call(..) | Expr::Match(..) => None,
        }
    }

    /// Combine the constants in a left-associated monus or quotient.
    ///
    /// Natural monus is truncated at zero, and division by zero answers zero.
    /// Those definitions make both directional laws total, including where
    /// either constant is zero:
    ///
    /// ```text
    /// (x monus a) monus b = x monus (a + b)
    /// (x / a) / b         = x / (a * b)
    /// ```
    fn chained(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        let (op @ (Monus | Div), &[inner, outer]) = (self.op(called)?, args) else {
            return None;
        };
        self.constant(scope, outer)?;
        let (_, inner_op, [dynamic, inner_constant]) = self.op_call(scope, inner)?;
        self.constant(scope, inner_constant)
            .filter(|_| inner_op == op)?;
        let combine = if op == Monus { Add } else { Mul };
        let combined = self.calculate(combine, called, inner_constant, outer, scope)?;
        Some(Again(Called(called, vec![dynamic, combined])))
    }

    /// Combine constants across one nested associative operation.
    ///
    /// Calls are A-normal, so `3 + (x + 5)` is an outer call whose second
    /// argument names an earlier call. Recognition proves both calls are the
    /// same pure, total operation; rewriting them neither drops nor reorders
    /// observable work. Constants are combined under the same bounded
    /// evaluator as ordinary folding.
    fn associated(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        let (op @ (Add | Mul), &[left, right]) = (self.op(called)?, args) else {
            return None;
        };
        let (inner, constant, swapped) = self.one_constant(scope, left, right)?;
        // Where nothing combines, a lone constant goes on the right.
        let commuted = swapped.then(|| Again(Called(called, vec![inner, constant])));
        let Some((_, inner_op, [l, r])) = self.op_call(scope, inner) else {
            return commuted;
        };
        if inner_op != op {
            return commuted;
        }
        let (dynamic, inner_constant, _) = self.one_constant(scope, l, r)?;
        let combined = self.calculate(op, called, constant, inner_constant, scope)?;
        Some(Again(Called(called, vec![dynamic, combined])))
    }

    /// Evaluate two known operands under this pass's shared folding budget.
    fn calculate(
        &mut self,
        op: Operation,
        called: FnId,
        left: Atom,
        right: Atom,
        scope: &mut Scope,
    ) -> Option<Atom> {
        let (left, right) = (self.constant(scope, left)?, self.constant(scope, right)?);
        let budget = &mut self.budget;
        let left = self.program.constant(left).to_owned(budget).ok()?;
        let right = self.program.constant(right).to_owned(budget).ok()?;
        let value = Evaluator {
            program: self.program,
            operations: &self.recognised.operations,
        }
        .arithmetic(
            op,
            self.program.function(called).result,
            &[left, right],
            budget,
        )
        .ok()?;
        let value = self.program.intern_held(&value);
        Some(self.push(Expr::Static(value), scope))
    }

    /// What `evaluate` answers for `args`, written as a known value, where
    /// the walk evaluates and every one of `args` is known. A refusal --
    /// an input known only as it runs, a call with no body, or the budget --
    /// leaves the expression to be written as it was.
    fn evaluated(
        &mut self,
        args: &[Atom],
        scope: &mut Scope,
        evaluate: impl FnOnce(&Evaluator, Vec<Held>, &mut Budget) -> Evaluation,
    ) -> Option<Atom> {
        if !self.rules.evaluate || self.budget.remaining == 0 {
            return None;
        }
        let known = args
            .iter()
            .map(|&atom| self.known(atom))
            .collect::<Option<Vec<_>>>()?;
        let budget = &mut self.budget;
        let values = known
            .into_iter()
            .map(|value| self.program.constant(value).to_owned(budget))
            .collect::<Result<Vec<_>, _>>()
            .ok()?;
        let evaluator = Evaluator {
            program: self.program,
            operations: &self.recognised.operations,
        };
        let value = evaluate(&evaluator, values, budget).ok()?;
        let value = self.program.intern_held(&value);
        Some(self.push(Expr::Static(value), scope))
    }

    /// A match on a known value [`Self::selected`] does not take apart, a
    /// sequence, evaluated whole, where the walk evaluates: the arm it takes
    /// may read only what is known. Opening the sequence instead would hold
    /// its rest as a second known value, a copy of all but one element.
    fn evaluated_match(
        &mut self,
        id: ExprId,
        scrutinee: Atom,
        locals: &[Atom],
        scope: &mut Scope,
    ) -> Option<Atom> {
        if !self.rules.evaluate || self.budget.remaining == 0 {
            return None;
        }
        self.known(scrutinee)?;
        // Only what the match reads from outside it is worth copying out.
        let mut read = vec![false; locals.len()];
        for expr in super::walk::nested(self.program, [id]) {
            for atom in super::walk::operands(self.program, expr) {
                if let Some(read) = read.get_mut(atom.0.index()) {
                    *read = true;
                }
            }
        }
        let known: Vec<_> = locals
            .iter()
            .zip(read)
            .map(|(&atom, read)| self.known(atom).filter(|_| read))
            .collect();
        let budget = &mut self.budget;
        let mut env = Vec::with_capacity(known.len());
        for value in known {
            let held = match value {
                Some(value) => Ok(self.program.constant(value).to_owned(budget).ok()?),
                None => Err(CannotFold::DynamicInput),
            };
            env.push(held);
        }
        let evaluator = Evaluator {
            program: self.program,
            operations: &self.recognised.operations,
        };
        let value = evaluator.expr(id, &mut env, budget).ok()?;
        let value = self.program.intern_held(&value);
        Some(self.push(Expr::Static(value), scope))
    }

    /// A call of `called` with `args` as [`Self::shared`] compares it, where
    /// the walk shares and `called` is answered rather than walked: pure and
    /// total, so a second call with the same operands answers the same.
    /// Operands known as values are compared by what they are, and those of
    /// a commutative operation in order.
    fn sharing(&self, called: FnId, args: &[Atom]) -> Option<Call> {
        if !self.rules.share || !(self.recognised.answered(called) || self.shareable(called)) {
            return None;
        }
        let mut operands: Vec<_> = args
            .iter()
            .map(|&atom| {
                self.known(atom).map_or_else(
                    || Operand::Local(atom.0.index()),
                    |value| {
                        let constant = self.program.constant(value);
                        Operand::Value(constant.owner(), constant.encoded().to_vec())
                    },
                )
            })
            .collect();
        if matches!(
            self.recognised.operations[called.index()],
            Some(Add | Mul | And | Or | Xor | Equals { .. })
        ) {
            operands.sort();
        }
        Some(Call(called, operands))
    }

    /// The local an earlier call the same as `call` was written to, where
    /// the binding that wrote it is still in scope: below the next level
    /// `scope` binds, and written there still or written over only by what
    /// an arm knows it to be.
    fn shared(&self, call: &Call, scope: &Scope) -> Option<Atom> {
        let &(atom, wrote) = self.calls.get(call)?;
        self.stands(atom, wrote, scope).then_some(atom)
    }

    /// Whether a call of `called`, which is not answered, answers the same
    /// as one the same before it: it reaches nothing outside, and answers
    /// no structure holding itself. A tree two calls build is two trees as
    /// written, though equal, and sharing one would build half of them.
    fn shareable(&self, called: FnId) -> bool {
        self.pure.get(called.index()) == Some(&true)
            && !self.holds_itself(self.program.function(called).result)
    }

    /// Whether a value of `ty` may hold another of it, as a list or a tree
    /// does; a natural is a number, however it is written.
    fn holds_itself(&self, ty: super::TypeId) -> bool {
        !matches!(
            super::type_shape(self.program, ty),
            super::TypeShape::Natural { .. }
        ) && self
            .program
            .ctors(ty)
            .iter()
            .any(|ctor| self.program.types_of(ctor.fields).contains(&ty))
    }

    /// Whether `wrote` still stands as the local `atom` in `scope`: below
    /// the next level it binds, and written there still or written over
    /// only by what an arm knows it to be.
    fn stands(&self, atom: Atom, wrote: ExprId, scope: &Scope) -> bool {
        let level = atom.0.index();
        level < scope.level && self.written.get(level) == Some(&Some(wrote))
            || self.shadowed.contains(&(level, wrote))
    }

    /// The known value a local was written as, in this scope or one
    /// around it.
    fn known(&self, atom: Atom) -> Option<super::ConstId> {
        let Expr::Static(value) = self.written_as(atom)? else {
            return None;
        };
        Some(value)
    }

    /// Which constant wrote a local, if one did.
    fn constant(&self, scope: &Scope, atom: Atom) -> Option<super::ConstId> {
        let Expr::Static(value) = self.program.expr(scope.wrote(atom)?) else {
            return None;
        };
        Some(value)
    }

    /// A sum or a product taken modulo a power of two, worked out modulo
    /// that power of two throughout.
    ///
    /// Adding and multiplying carry through a remainder: what two numbers
    /// come to modulo `m` is what their remainders come to modulo `m`. So a
    /// program asking for the low bits of a sum is asking for a sum of low
    /// bits, and nothing in between it and the answer has to be any wider
    /// than the answer is.
    ///
    /// This is the whole of how a language whose only number counts without
    /// a last step says eight-bit wrapping arithmetic. Written the other way
    /// round it says the same and means two arbitrary-precision numbers, a
    /// heap to hold them, and a division to finish.
    ///
    /// Only a power of two: any other modulus leaves three divisions where
    /// there was one. And not truncated subtraction, which stops at nought
    /// rather than carrying through, nor division or the remainder itself.
    fn reduced(&mut self, scope: &mut Scope, site: &Called) -> Option<Step> {
        let (called, args) = site.parts();
        let (Mod, &[held, modulus]) = (self.op(called)?, args) else {
            return None;
        };
        self.number(scope, modulus)
            .filter(|m| m.is_power_of_two())?;
        let (inner, Add | Mul, [left, right]) = self.op_call(scope, held)? else {
            return None;
        };
        // A side already within the modulus is left alone, and where both
        // are there is nothing to do: saying it twice says no more, and
        // this walk happens more than once.
        if self.within(scope, left, modulus) && self.within(scope, right, modulus) {
            return None;
        }
        let low = self.remainder(called, left, modulus, scope);
        // One side reduced once where both sides are the same side.
        let high = if left == right {
            low
        } else {
            self.remainder(called, right, modulus, scope)
        };
        let sum = self.emit(inner, &[low, high], scope);
        Some(Again(Called(called, vec![sum, modulus])))
    }

    /// `side` modulo `modulus`, unless it is already within it.
    fn remainder(&mut self, called: FnId, side: Atom, modulus: Atom, scope: &mut Scope) -> Atom {
        if self.within(scope, side, modulus) {
            side
        } else {
            self.emit(called, &[side, modulus], scope)
        }
    }

    /// Whether a side of a sum is already every bit of itself the modulus
    /// keeps, and so needs no remainder of its own.
    ///
    /// Two ways of being that, and both have to be asked. A side written as a
    /// remainder by this same modulus has been round this law already, which
    /// is what stops it nesting one inside another; and a side known to be
    /// under the modulus before the program runs was never going to lose
    /// anything to it. Asking only the first wraps a constant in a remainder
    /// that cannot do anything, and what the tier below reads then is a
    /// number widened to the modulus and narrowed back.
    fn within(&self, scope: &Scope, atom: Atom, modulus: Atom) -> bool {
        let reduced = matches!(self.op_call(scope, atom), Some((_, Mod, [_, m])) if m == modulus);
        let under = (self.number(scope, atom))
            .zip(self.number(scope, modulus))
            .is_some_and(|(held, limit)| held < limit);
        reduced || under
    }

    /// The number a local is, where it is one known before the program runs
    /// and small enough to work out with.
    fn number(&self, scope: &Scope, atom: Atom) -> Option<u64> {
        let value = self.constant(scope, atom)?;
        let super::ConstantView::Natural(number) = self.program.constant(value).view() else {
            return None;
        };
        number.try_u64().ok()
    }

    /// The recognised arithmetic operation `called` is.
    fn op(&self, called: FnId) -> Option<Operation> {
        self.recognised.operations[called.index()]
    }

    /// The recognised operation of two operands a local was written as a
    /// call of, in this scope: what it called, the operation, the operands.
    fn op_call(&self, scope: &Scope, atom: Atom) -> Option<(FnId, Operation, [Atom; 2])> {
        let Expr::Call(called, args) = self.program.expr(scope.wrote(atom)?) else {
            return None;
        };
        let &[a, b] = self.program.atoms(args) else {
            return None;
        };
        Some((called, self.op(called)?, [a, b]))
    }

    /// Of two operands where exactly one was written as a constant in this
    /// scope: the other, the constant, and whether the constant came first.
    fn one_constant(&self, scope: &Scope, a: Atom, b: Atom) -> Option<(Atom, Atom, bool)> {
        match (self.constant(scope, a), self.constant(scope, b)) {
            (Some(_), None) => Some((b, a, true)),
            (None, Some(_)) => Some((a, b, false)),
            _ => None,
        }
    }

    /// The number `n`, of the type `called` answers, written down.
    fn number_of(&mut self, called: FnId, n: u64, scope: &mut Scope) -> Option<Atom> {
        let result = self.program.function(called).result;
        let number = self.program.intern_number(result, n).ok()?;
        Some(self.push(Expr::Static(number), scope))
    }

    /// What wrote a local, in this scope or one around it, where a binding
    /// did.
    fn written_as(&self, atom: Atom) -> Option<Expr> {
        let wrote = (*self.written.get(atom.0.index())?)?;
        Some(self.program.expr(wrote))
    }

    /// A constructor of no fields, written down.
    fn nullary(&mut self, ctor: super::CtorId, scope: &mut Scope) -> Atom {
        self.push(Expr::Con(ctor, Range::default()), scope)
    }

    /// A call of `called` with `args`, written down.
    fn emit(&mut self, called: FnId, args: &[Atom], scope: &mut Scope) -> Atom {
        let args = self.held(args);
        self.push(Expr::Call(called, args), scope)
    }

    fn held(&mut self, atoms: &[Atom]) -> Range {
        self.program.terms.hold_atoms(atoms.iter().copied())
    }

    /// Write an expression down and answer the local it took.
    fn push(&mut self, expr: Expr, scope: &mut Scope) -> Atom {
        let id = self.program.terms.push(expr);
        scope.bindings.push(id);
        let held = Atom(Local::at(scope.level));
        self.written.truncate(scope.level);
        self.written.resize(scope.level, None);
        self.written.push(Some(id));
        scope.level += 1;
        held
    }

    fn callee(&mut self, called: FnId) -> Option<Body> {
        if self.recognised.answered(called) {
            return None;
        }
        self.inlining
            .as_mut()?
            .callee(self.program, called, self.inline_depth)
    }

    fn args(&self, args: Range, locals: &[Atom]) -> Vec<Atom> {
        self.program
            .atoms(args)
            .iter()
            .map(|a| locals[a.0.index()])
            .collect()
    }
}
