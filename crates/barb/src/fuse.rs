//! Fusing a walk that builds a list with the walk that takes it apart.
//!
//! A producer walks a list and pushes one element onto an accumulator for
//! each it passes, or none, then answers the accumulator reversed: the
//! elements in the order it made them. `map` and `trail` are written this
//! way, `filter` pushing an element only where it keeps it, and so is
//! `range`, whose walk is down a number rather than a list: a number is a
//! list of nothing but its length. One pushing the same value every
//! round, as `repeat` does, may answer the accumulator as it stands, since
//! from either end it built the same list. A
//! consumer walks a list from the front, reading each element once and
//! going on with nothing but the rest, in tail position: `fold`, `trail`,
//! `find_index`. Where a producer's answer is read once, by a consumer, the
//! list between them need not exist. One walk over what the producer walks
//! does both steps, handing the consumer each element as the producer makes
//! it, and stops where the consumer would have stopped.
//!
//! A consumer may hand the rest of its list to another consumer rather than
//! itself, as `min` takes the first element and folds the rest, and the
//! fused walk then goes on as the producer fused with that one. A match
//! taking apart a producer's list where it is built consumes it as a
//! consumer's call would, and is read as a definition of its own.
//!
//! The consumer may itself be a producer, as `trail` is, and then so is the
//! walk the two become. A chain of them therefore fuses a link at a time,
//! within one pass over each body.
//!
//! Nothing is recognized by name. A producer and a consumer are shapes of
//! equation, and a list is a type of sequence shape. Only definitions that
//! reach no bodyless call take part: fusing interleaves the two walks'
//! work, and skips what the producer would have made past where the
//! consumer stops, which only work nothing can observe may have done to it.

use rustc_hash::FxHashMap as HashMap;

use super::copy::{self, Scope};
use super::facts::Recognised;
use super::walk;
use super::{Arm, Atom, Body, Expr, ExprId, FnId, Local, Program, SequenceOperation, TypeId};

/// Fuse every producer read once by a consumer, in every body.
pub(crate) fn fuse(program: &mut Program) {
    let recognised = program.recognised().clone();
    let mut fusion = Fusion {
        pure: super::calls::pure(program, &recognised),
        recognised,
        producers: HashMap::default(),
        consumers: HashMap::default(),
        fused: HashMap::default(),
        prepends: HashMap::default(),
        within: FnId::at(0),
        reading: Vec::new(),
        program,
    };
    let mut bodies = Vec::new();
    let count = fusion.program.functions().len();
    for at in 0..count {
        let id = FnId::at(at);
        if fusion.recognised.answered(id) {
            continue;
        }
        if let Some(rewritten) = fusion.rewrite_function(id) {
            bodies.push((id, rewritten));
        }
    }
    fusion.program.replace_bodies(bodies);
    // A fused walk is copied from its two, and what they called is what it
    // calls: a chain written inside a step, as a walk recursing through a
    // `map` writes it, is fused there too, each made walk read in turn.
    let mut read = count;
    while read < fusion.program.functions().len() {
        let id = FnId::at(read);
        read += 1;
        if !fusion.fused.values().any(|&made| made == id) {
            continue;
        }
        if let Some(rewritten) = fusion.rewrite_function(id) {
            fusion.program.replace_bodies(vec![(id, rewritten)]);
            fusion.producers.remove(&id);
            fusion.consumers.remove(&id);
        }
    }
    // A match read as a walk of its own is a call where it stood, which
    // inlining puts back where it can: one taking the first element of what
    // a walk builds goes round nowhere but in what it calls.
    fusion.reading.dedup();
    super::normalize::reinline(fusion.program, &fusion.reading);
}

/// A walk building a list, as [`Fusion::producer`] reads it.
#[derive(Clone, Copy)]
struct Producer {
    /// The parameter it walks, and the one it pushes onto.
    list: usize,
    acc: usize,
}

/// A walk taking a list apart, as [`Fusion::consumer`] reads it.
#[derive(Clone, Copy)]
struct Consumer {
    list: usize,
}

/// A fused walk, and where its producer's and consumer's parameters are.
struct Pair {
    id: FnId,
    /// The list the producer walks, which the fused walk walks.
    list: usize,
    /// The producer's accumulator and the consumer's list, which the fused
    /// walk has no parameter for.
    acc: usize,
    taken: usize,
    consumer: FnId,
    /// The fused walk's parameter each of theirs is, by position.
    p_locals: Vec<Atom>,
    c_locals: Vec<Atom>,
    arity: usize,
    /// Each consumer the consumer's step hands the rest of its list to, the
    /// parameter it takes it as, and the walk fusing the producer into it.
    onward: Vec<(FnId, usize, FnId)>,
}

/// A producer's call, held back until its one reader is written.
#[derive(Clone)]
struct Deferred {
    callee: FnId,
    /// Its arguments as the source wrote them, and as rewritten.
    written: Vec<Atom>,
    args: Vec<Atom>,
}

struct Fusion<'a> {
    program: &'a mut Program,
    recognised: Recognised,
    /// Indexed by definition: see [`super::calls::pure`].
    pure: Vec<bool>,
    producers: HashMap<FnId, Option<Producer>>,
    consumers: HashMap<FnId, Option<Consumer>>,
    fused: HashMap<(FnId, FnId), FnId>,
    /// The definition putting one sequence of each type before another in
    /// order, found or written the first time it is wanted.
    prepends: HashMap<TypeId, FnId>,
    /// The definition whose body is being rewritten, and each in which a
    /// match was read as a walk of its own.
    within: FnId,
    reading: Vec<FnId>,
}

/// A local no fused body may read: what a parameter or field the fused walk
/// has no use for stands for, so that reading one fails validation rather
/// than reading something else.
const UNREAD: Atom = Atom(Local(u32::MAX));

/// What the consumer's step reads as the rest of its list, which it reads
/// only to go round on, and going round is the fused walk's call instead:
/// no fused body keeps it.
const REST: Atom = Atom(Local(u32::MAX - 1));

impl Fusion<'_> {
    /// The body of `id` with each producer it calls read once by a
    /// consumer fused, where it calls one.
    fn rewrite_function(&mut self, id: FnId) -> Option<Body> {
        let body = self.program.function(id).body()?;
        if !self.calls_a_producer(body) {
            return None;
        }
        let types = self.program.params(id).to_vec();
        let locals = (0..types.len()).map(|at| Atom(Local::at(at))).collect();
        self.within = id;
        Some(self.rewrite_body(body, locals, types))
    }

    /// Whether anything in a body calls a producer, which is all that makes
    /// rewriting it worth a copy.
    fn calls_a_producer(&mut self, body: Body) -> bool {
        #[expect(
            clippy::needless_collect,
            reason = "asking whether a callee is a producer caches the answer, which the walk's borrow forbids"
        )]
        let callees: Vec<FnId> = walk::within(self.program, body)
            .filter_map(|id| match self.program.expr(id) {
                Expr::Call(callee, _) => Some(callee),
                _ => None,
            })
            .collect();
        callees
            .into_iter()
            .any(|callee| self.producer(callee).is_some() || self.reversal(callee))
    }

    /// Whether `id` is a producer:
    ///
    /// ```text
    /// p(.. list .. acc ..) = match list
    ///   empty => ...; reverse(acc, <empty>)
    ///   push(x, rest) => ...; next = push(e, acc); p(.. next ..)
    /// ```
    ///
    /// where `acc` is read nowhere else, and `reverse` is a reversal; or the
    /// same building front to back, as [`super::forward`] writes a walk,
    /// `append(acc, e)` for the push and `prepend(acc, <empty>)`, or `acc`
    /// itself, at the end. What it goes on walking is its own: the rest, or
    /// whatever its step worked out. Its call may stand inside matches
    /// ending the step, each way through them pushing and going on, or
    /// going on with `acc` as it came, as a filter does, so long as one
    /// pushes. Built front to back, a way may instead stop, answering `acc`
    /// as the empty arm does, as `map2` stops where its second list runs
    /// out. What it walks may be a number, `zero` and
    /// `succ(rest)` for `empty` and `push(x, rest)`; and where it pushes a
    /// parameter it hands itself as it came, it may answer `acc` itself
    /// having pushed in front.
    fn producer(&mut self, id: FnId) -> Option<Producer> {
        if let Some(&known) = self.producers.get(&id) {
            return known;
        }
        let found = self.find_producer(id);
        self.producers.insert(id, found);
        found
    }

    #[expect(
        clippy::too_many_lines,
        reason = "each condition of the producer shape is one early return, read in order"
    )]
    fn find_producer(&self, id: FnId) -> Option<Producer> {
        if !self.takes_part(id) {
            return None;
        }
        let body = self.program.function(id).body()?;
        let (list, arms) = walk::top_match(self.program, body)?;
        let params = self.program.params(id);
        let (empty, push, _) = walked(self.program, params[list])?;
        let arity = params.len();
        let [nil_arm, push_arm] = walk::split_arms(self.program, arms, empty, push)?;
        // The empty arm answers the accumulator reversed or prepended onto
        // nothing -- or, built front to back, the accumulator itself, which
        // is what prepending it onto nothing answers.
        let (acc, ended) = match self.program.expr(nil_arm.body.tail) {
            Expr::Atom(Atom(acc)) => (acc.index(), None),
            Expr::Call(reverse, args) => {
                let &[Atom(acc), Atom(onto)] = self.program.atoms(args) else {
                    return None;
                };
                (acc.index(), Some((reverse, onto)))
            }
            _ => return None,
        };
        if acc >= arity || acc == list {
            return None;
        }
        // What it builds is a list too, but not necessarily of what it walks.
        let (built_empty, built_push, built_recursive) =
            super::sequence::sequence(self.program, params[acc])?;
        if let Some((reverse, onto)) = ended {
            let ends = super::sequence::reverses(self.program, reverse)
                || self.recognised.sequences.get(reverse.index())
                    == Some(&Some(SequenceOperation::Prepend));
            let onto = walk::bound(self.program, nil_arm.body, arity, onto.index())?;
            if !ends || !self.empty_constant(onto, built_empty) {
                return None;
            }
        }
        // Each way through the push arm, down whatever matches it ends
        // with, either pushes one element onto it and goes on, or goes on
        // with it as it came, pushing nothing: a filter's step. What it
        // goes on walking is its own affair: the consumer is handed what
        // it pushed, wherever that came from. Built front to back, a way
        // may also stop early, answering the accumulator as the empty arm
        // does: `map2` running out of its second list.
        let push_body = push_arm.body;
        let fields = self.program.fields(push).len();
        let mut ways = Vec::new();
        let mut stops = 0;
        let stopping = ended.is_none().then_some((acc, &mut stops));
        self.ways(id, push_body, arity + fields, &mut ways, stopping)?;
        let reversed =
            ended.is_some_and(|(reverse, _)| super::sequence::reverses(self.program, reverse));
        let mut pushing_ways = 0;
        for &(inner, base, args) in &ways {
            let args = self.program.atoms(args);
            if args[acc].0.index() == acc {
                continue;
            }
            pushing_ways += 1;
            let pushed = walk::bound(self.program, inner, base, args[acc].0.index())?;
            // Answered reversed, it must push in front; answered as it
            // stands, it must have been built in order -- or pushed the one
            // value every time, handed on as it came, so that whichever end
            // it was built from is the same list: a repeat.
            let element = self.pushed_element(pushed, acc, (built_push, built_recursive))?;
            let same = element.0.index() < arity && args[element.0.index()] == element;
            let in_order = if reversed {
                !self.appends(pushed)
            } else {
                self.appends(pushed) || (same && ways.len() == 1)
            };
            // Nothing else reads the list it pushed onto.
            if !in_order || self.reads_after(inner, base, args[acc].0.index()) != 1 {
                return None;
            }
        }
        // Nothing else reads the accumulator: each way reads it once.
        let reads = self.reads(push_body, acc) + self.reads(nil_arm.body, acc);
        (pushing_ways > 0 && reads == ways.len() + stops + 1).then_some(Producer { list, acc })
    }

    /// Each way through a producer's step `body`, whose first binding is at
    /// `base`, down the matches it ends with to a call of `id` itself: the
    /// body ending in the call, its first binding's level, and the call's
    /// arguments. A way answering the parameter `stopping` names, counted
    /// there, stops early; nothing where a way ends otherwise.
    fn ways(
        &self,
        id: FnId,
        body: Body,
        base: usize,
        ways: &mut Vec<(Body, usize, super::Range)>,
        mut stopping: Option<(usize, &mut usize)>,
    ) -> Option<()> {
        let inner = base + self.program.bindings(body.bindings).len();
        match self.program.expr(body.tail) {
            Expr::Call(again, args)
                if again == id
                    && self.program.atoms(args).len() == self.program.params(id).len() =>
            {
                ways.push((body, base, args));
                Some(())
            }
            Expr::Atom(Atom(answer)) => {
                let (acc, stops) = stopping?;
                (answer.index() == acc).then(|| *stops += 1)
            }
            Expr::Match(_, arms) => {
                for arm in self.program.arms(arms).to_vec() {
                    let fields = self.program.fields(arm.ctor).len();
                    let again = stopping.as_mut().map(|(acc, stops)| (*acc, &mut **stops));
                    self.ways(id, arm.body, inner + fields, ways, again)?;
                }
                Some(())
            }
            _ => None,
        }
    }

    /// The element `expr` puts onto the accumulator at level `acc`: a push
    /// in front of it, or an append after it.
    fn pushed_element(
        &self,
        expr: ExprId,
        acc: usize,
        (push, recursive): (super::CtorId, usize),
    ) -> Option<Atom> {
        match self.program.expr(expr) {
            Expr::Con(ctor, fields) => {
                let fields = self.program.atoms(fields);
                (ctor == push && fields[recursive].0.index() == acc).then(|| fields[1 - recursive])
            }
            Expr::Call(callee, args) => {
                let &[onto, element] = self.program.atoms(args) else {
                    return None;
                };
                let appends = self.recognised.sequences.get(callee.index())
                    == Some(&Some(SequenceOperation::Append));
                (appends && onto.0.index() == acc).then_some(element)
            }
            Expr::Atom(_) | Expr::Static(_) | Expr::Match(..) => None,
        }
    }

    /// Whether `expr` appends to what it builds, rather than pushing in
    /// front of it.
    fn appends(&self, expr: ExprId) -> bool {
        matches!(
            self.program.expr(expr),
            Expr::Call(callee, _) if self.recognised.sequences.get(callee.index())
                == Some(&Some(SequenceOperation::Append))
        )
    }

    /// Whether a definition may be fused at all: it reaches no bodyless
    /// call, and is not answered as an operation it was recognized as.
    /// A definition this pass made was neither.
    fn takes_part(&self, id: FnId) -> bool {
        self.pure[id.index()]
            && (id.index() >= self.recognised.operations.len() || !self.recognised.answered(id))
    }

    /// Whether `id` is a consumer:
    ///
    /// ```text
    /// c(.. list ..) = match list
    ///   empty => ...
    ///   push(x, rest) => ... c(.. rest ..) ... d(.. rest ..) ...
    /// ```
    ///
    /// where `list` is read only by the match, and `rest` only as the list
    /// a consumer takes, `c` itself or another, at most once on any way
    /// through the step: as `min` takes the first element and folds the
    /// rest. A definition asked again while it is read is taken as none.
    fn consumer(&mut self, id: FnId) -> Option<Consumer> {
        if let Some(&known) = self.consumers.get(&id) {
            return known;
        }
        self.consumers.insert(id, None);
        let found = self.find_consumer(id);
        self.consumers.insert(id, found);
        found
    }

    fn find_consumer(&mut self, id: FnId) -> Option<Consumer> {
        if !self.takes_part(id) {
            return None;
        }
        let body = self.program.function(id).body()?;
        let (list, arms) = walk::top_match(self.program, body)?;
        let params = self.program.params(id);
        let taken = (params[list], params.len());
        self.consumes(arms, taken, list, Some(id))
            .then_some(Consumer { list })
    }

    /// Whether a match taking apart local `list`, a sequence of type `ty`
    /// whose fields an arm binds from level `at`, consumes it as
    /// [`Self::consumer`] says. `own`, the definition the match is all of,
    /// takes `list` as its parameter of that position.
    fn consumes(
        &mut self,
        arms: super::Range,
        (ty, at): (TypeId, usize),
        list: usize,
        own: Option<FnId>,
    ) -> bool {
        let Some((empty, push, recursive)) = super::sequence::sequence(self.program, ty) else {
            return false;
        };
        let Some([nil_arm, push_arm]) = walk::split_arms(self.program, arms, empty, push) else {
            return false;
        };
        if self.reads(nil_arm.body, list) + self.reads(push_arm.body, list) != 0 {
            return false;
        }
        let rest = at + recursive;
        let handed = self.handed(push_arm.body, rest, own.map(|id| (id, list)));
        handed.len() == self.reads(push_arm.body, rest) && self.most_reads(push_arm.body, rest) <= 1
    }

    /// Each consumer `body` hands local `rest` to as its list, once a call,
    /// `own` counting as one taking its list at that position, and no other
    /// that is a producer too.
    fn handed(&mut self, body: Body, rest: usize, own: Option<(FnId, usize)>) -> Vec<FnId> {
        let calls: Vec<_> = walk::within(self.program, body)
            .filter_map(|id| match self.program.expr(id) {
                Expr::Call(callee, args) => Some((callee, self.program.atoms(args).to_vec())),
                _ => None,
            })
            .collect();
        let mut handed = Vec::new();
        for (callee, args) in calls {
            // Another that builds a list in turn is left alone: what it
            // builds may be read by a walk fused again, and so on forever.
            let taken = match own {
                Some((id, list)) if id == callee => Some(list),
                _ if self.producer(callee).is_some() => None,
                _ => self.consumer(callee).map(|Consumer { list }| list),
            };
            let once = args.iter().filter(|atom| atom.0.index() == rest).count() == 1;
            if once && taken.is_some_and(|list| args[list].0.index() == rest) {
                handed.push(callee);
            }
        }
        handed
    }

    /// The most times any one way through `body` reads local `level`.
    fn most_reads(&self, body: Body, level: usize) -> usize {
        let exprs = self
            .program
            .bindings(body.bindings)
            .iter()
            .chain([&body.tail]);
        exprs
            .map(|&id| {
                let arms = match self.program.expr(id) {
                    Expr::Match(_, arms) => self.program.arms(arms).iter(),
                    _ => [].iter(),
                };
                let own = walk::operands(self.program, id).filter(|atom| atom.0.index() == level);
                own.count()
                    + arms
                        .map(|arm| self.most_reads(arm.body, level))
                        .max()
                        .unwrap_or(0)
            })
            .sum()
    }

    /// Whether an expression is the empty list: `empty` applied, or a known
    /// value built by it, which is what folding makes of the first.
    fn empty_constant(&self, expr: ExprId, empty: super::CtorId) -> bool {
        match self.program.expr(expr) {
            Expr::Con(ctor, _) => ctor == empty,
            Expr::Static(value) => self.program.constant(value).constructor() == empty,
            Expr::Atom(_) | Expr::Call(..) | Expr::Match(..) => false,
        }
    }

    /// How many times `body` reads local `level`, which is bound before any
    /// of it: in its bindings, its tail, and every arm in them. Arms bind
    /// only levels past it, so a read of `level` anywhere is this one.
    fn reads(&self, body: Body, level: usize) -> usize {
        walk::reads(self.program, walk::within(self.program, body), level)
    }

    /// How many times the bindings of `body` after its binding at `level`,
    /// and its tail, read that binding. Its first binding takes `base`. An
    /// arm of an earlier binding may bind the same level for itself.
    fn reads_after(&self, body: Body, base: usize, level: usize) -> usize {
        let later = &self.program.bindings(body.bindings)[level + 1 - base..];
        let exprs = walk::nested(self.program, later.iter().copied().chain([body.tail]));
        walk::reads(self.program, exprs, level)
    }

    /// The function walking what `producer` walks and doing what `consumer`
    /// does with each element it would have pushed, made the first time it
    /// is asked for.
    ///
    /// Its parameters are the producer's but its accumulator, then the
    /// consumer's but its list.
    fn fused(&mut self, producer: FnId, consumer: FnId) -> FnId {
        if let Some(&id) = self.fused.get(&(producer, consumer)) {
            return id;
        }
        let mut pair = self.declared(producer, consumer);
        self.fused.insert((producer, consumer), pair.id);
        self.fuse_pair(producer, consumer, &mut pair);
        pair.id
    }

    fn fuse_pair(&mut self, producer: FnId, consumer: FnId, pair: &mut Pair) {
        let p_body = self.program.function(producer).body().expect("a producer");
        let c_body = self.program.function(consumer).body().expect("a consumer");
        let list = pair.list;
        let (_, p_arms) = walk::top_match(self.program, p_body).expect("a producer's match");
        let (_, c_arms) = walk::top_match(self.program, c_body).expect("a consumer's match");
        let walks = self.program.params(producer)[list];
        let (empty, push, _) = walked(self.program, walks).expect("a list or a number");
        let taken = self.program.params(consumer)[pair.taken];
        let (c_empty, c_push, _) = super::sequence::sequence(self.program, taken).expect("a list");
        let [_, p_step] = walk::split_arms(self.program, p_arms, empty, push).expect("two arms");
        let [c_nil, c_step] =
            walk::split_arms(self.program, c_arms, c_empty, c_push).expect("two arms");
        // Each consumer the step hands the rest to goes on as the walk
        // fusing the producer into it.
        let (_, _, recursive) = super::sequence::sequence(self.program, taken).expect("a list");
        let rest = self.program.params(consumer).len() + recursive;
        for onward in self.handed(c_step.body, rest, Some((consumer, pair.taken))) {
            let Consumer { list } = self.consumers[&onward].expect("a consumer");
            let fused = self.fused(producer, onward);
            pair.onward.push((onward, list, fused));
        }
        let mut arms = Vec::new();
        for arm in self.program.arms(p_arms).to_vec() {
            let body = if arm.ctor == empty {
                // No more elements: the consumer is handed an empty list.
                let locals = pair.c_locals.clone();
                copy::body(self.program, c_nil.body, locals, pair.arity, None)
            } else {
                self.step(pair, p_step, (c_step, c_nil))
            };
            arms.push(Arm {
                ctor: arm.ctor,
                body,
            });
        }
        let arms = self.program.terms.hold_arms(&arms);
        let tail = self
            .program
            .terms
            .push(Expr::Match(pair.p_locals[list], arms));
        let body = self.program.terms.hold_body(&[], tail);
        self.program.replace_bodies(vec![(pair.id, body)]);
    }

    /// Declare the walk fusing `producer` into `consumer`, and say where each
    /// of their parameters is in it.
    fn declared(&mut self, producer: FnId, consumer: FnId) -> Pair {
        let Producer { list, acc } = self.producers[&producer].expect("a producer");
        let Consumer { list: taken } = self.consumers[&consumer].expect("a consumer");
        let mut params: Vec<_> = but(self.program.params(producer), acc).collect();
        let before = params.len();
        params.extend(but(self.program.params(consumer), taken));
        let name = format!(
            "{}+{}",
            self.program.name(self.program.function(producer).name),
            self.program.name(self.program.function(consumer).name)
        );
        let result = self.program.function(consumer).result;
        let id = self.program.declare(&name, &params, result);
        self.pure.push(true);
        // Each is the next of the fused walk's, but the one it has no use for.
        let placed = |count: usize, skip: usize, from: usize| -> Vec<Atom> {
            (0..count)
                .map(|at| match at.cmp(&skip) {
                    std::cmp::Ordering::Less => Atom(Local::at(from + at)),
                    std::cmp::Ordering::Equal => UNREAD,
                    std::cmp::Ordering::Greater => Atom(Local::at(from + at - 1)),
                })
                .collect()
        };
        Pair {
            id,
            list,
            acc,
            taken,
            consumer,
            p_locals: placed(self.program.params(producer).len(), acc, 0),
            c_locals: placed(self.program.params(consumer).len(), taken, before),
            arity: params.len(),
            onward: vec![(consumer, taken, id)],
        }
    }

    /// The fused walk's step: the producer's step, but for its push and its
    /// call, then the consumer's step on the element it would have pushed,
    /// going on with the fused walk wherever the consumer went on with
    /// itself, and ending as the consumer ends wherever the producer stops
    /// early.
    fn step(&mut self, pair: &Pair, p_step: Arm, (c_step, c_nil): (Arm, Arm)) -> Body {
        let arity = pair.arity;
        let fields = self.program.fields(p_step.ctor).len();
        let mut locals = pair.p_locals.clone();
        locals.extend((0..fields).map(|at| Atom(Local::at(arity + at))));
        self.step_body(
            pair,
            p_step.body,
            locals,
            arity + fields,
            (REST, c_step, c_nil),
        )
    }

    /// The producer's step `body`, whose locals so far `locals` maps, its
    /// first binding at `level` in the fused walk: down through each match
    /// it ends with, to each way that pushes and goes on, which goes on as
    /// the consumer's step instead, and each that goes on pushing nothing,
    /// which goes on as the fused walk with the consumer's state as it was,
    /// and each that stops, answering what it built, which ends as the
    /// consumer does on the empty list.
    #[expect(
        clippy::too_many_lines,
        reason = "each way a producer's step ends is one arm of the match on its tail"
    )]
    fn step_body(
        &mut self,
        pair: &Pair,
        body: Body,
        mut locals: Vec<Atom>,
        level: usize,
        consumer: (Atom, Arm, Arm),
    ) -> Body {
        let (rest, c_step, c_nil) = consumer;
        let mut scope = Scope::at(level);
        let tail = self.program.expr(body.tail);
        if let Expr::Call(_, again) = tail {
            let again = self.program.atoms(again);
            if again[pair.acc].0.index() != pair.acc {
                return self.pushing(pair, body, locals, scope, (rest, c_step));
            }
        }
        for binding in self.program.bindings(body.bindings).to_vec() {
            let value = copy::expr(self.program, binding, &locals, &mut scope, None);
            locals.push(value);
        }
        let answer = match tail {
            Expr::Call(_, again) => {
                let again = self.program.atoms(again).to_vec();
                self.skipping(pair, &again, &locals, &mut scope)
            }
            Expr::Match(on, arms) => {
                let mut rebuilt = Vec::new();
                for arm in self.program.arms(arms).to_vec() {
                    let fields = self.program.fields(arm.ctor).len();
                    let mut inner = locals.clone();
                    inner.extend((0..fields).map(|at| Atom(Local::at(scope.level + at))));
                    let body =
                        self.step_body(pair, arm.body, inner, scope.level + fields, consumer);
                    rebuilt.push(Arm {
                        ctor: arm.ctor,
                        body,
                    });
                }
                let arms = self.program.terms.hold_arms(&rebuilt);
                copy::push(
                    self.program,
                    &mut scope,
                    Expr::Match(locals[on.0.index()], arms),
                )
            }
            Expr::Atom(_) => {
                let locals = pair.c_locals.clone();
                copy::into(self.program, c_nil.body, locals, &mut scope, None)
            }
            _ => unreachable!("a producer's step ends in matches, then its call or its answer"),
        };
        copy::close(self.program, scope, answer)
    }

    /// The call going on where the producer pushed nothing, as it called
    /// itself with `again`: the fused walk, on what the producer goes on
    /// with and the consumer's state as it was.
    fn skipping(
        &mut self,
        pair: &Pair,
        again: &[Atom],
        locals: &[Atom],
        scope: &mut Scope,
    ) -> Atom {
        let mut going = Vec::with_capacity(pair.arity);
        going.extend(but(again, pair.acc).map(|atom| locals[atom.0.index()]));
        going.extend(but(&pair.c_locals, pair.taken));
        let args = self.program.terms.hold_atoms(going.into_iter());
        copy::push(self.program, scope, Expr::Call(pair.id, args))
    }

    /// The body where the producer pushes and goes on, copied into `scope`
    /// but for its push and its call, then the consumer's step on the
    /// element it would have pushed.
    #[expect(
        clippy::too_many_lines,
        reason = "the producer step, the element, and the consumer step are copied into one scope in order"
    )]
    fn pushing(
        &mut self,
        pair: &Pair,
        body: Body,
        mut locals: Vec<Atom>,
        mut scope: Scope,
        (rest, c_step): (Atom, Arm),
    ) -> Body {
        let (_, _, recursive) =
            super::sequence::sequence(self.program, self.program.params(pair.consumer)[pair.taken])
                .expect("a list");
        let Expr::Call(_, again) = self.program.expr(body.tail) else {
            unreachable!("a producer's step calls itself")
        };
        let again = self.program.atoms(again).to_vec();
        // The producer's own level of this body's first binding.
        let pushed = again[pair.acc].0.index() - locals.len();
        let bindings = self.program.bindings(body.bindings).to_vec();
        for (at, &binding) in bindings.iter().enumerate() {
            let value = if at == pushed {
                UNREAD
            } else {
                copy::expr(self.program, binding, &locals, &mut scope, None)
            };
            locals.push(value);
        }
        // What it pushed in front, or appended after.
        let element = match self.program.expr(bindings[pushed]) {
            Expr::Con(_, fields) => self.program.atoms(fields)[1 - recursive],
            Expr::Call(_, args) => self.program.atoms(args)[1],
            _ => unreachable!("a producer pushes or appends"),
        };
        let element = locals[element.0.index()];
        // What the producer goes on with, but for the list it is building:
        // that is what the consumer is handed instead.
        let going_on: Vec<_> = but(&again, pair.acc)
            .map(|atom| locals[atom.0.index()])
            .collect();
        let mut consumed = pair.c_locals.clone();
        consumed.extend((0..2).map(|at| if at == recursive { rest } else { element }));
        // A consumer handed the rest of the list, the consumer going round
        // or another, goes on as the walk fusing the producer into it, on
        // what the producer goes on with.
        let onward = &pair.onward;
        let mut redirect = |program: &mut Program, _: &mut Scope, _: ExprId, expr: Expr| {
            let Expr::Call(callee, args) = expr else {
                return expr;
            };
            let atoms = program.atoms(args).to_vec();
            let Some(&(_, taken, fused)) = onward
                .iter()
                .find(|&&(consumer, taken, _)| consumer == callee && atoms[taken] == rest)
            else {
                return expr;
            };
            let mut going = going_on.clone();
            going.extend(but(&atoms, taken));
            Expr::Call(fused, program.terms.hold_atoms(going.into_iter()))
        };
        let answer = copy::into(
            self.program,
            c_step.body,
            consumed,
            &mut scope,
            Some(&mut redirect),
        );
        copy::close(self.program, scope, answer)
    }

    /// A sequence reversed onto nothing, `made`, and reversed again onto
    /// another by `callee`, is put before that other in order.
    fn prepended(
        &mut self,
        callee: FnId,
        made: &Deferred,
        written: &[Atom],
        args: &[Atom],
    ) -> Deferred {
        debug_assert!(
            self.reversal(made.callee),
            "only a reversal is deferred for one"
        );
        let ty = self.program.params(callee)[0];
        Deferred {
            callee: self.prepending(ty, callee),
            written: vec![made.written[0], written[1]],
            args: vec![made.args[0], args[1]],
        }
    }

    /// Whether `id` is recognized as reversing one sequence onto another.
    fn reversal(&self, id: FnId) -> bool {
        self.recognised.sequences.get(id.index()) == Some(&Some(SequenceOperation::ReverseOnto))
    }

    /// Whether a reversal onto nothing is read once, by another reversal
    /// later in the same body, as what it reverses.
    fn reversed_twice(&self, call: &Deferred, body: Body, base: usize, level: usize) -> bool {
        let Some(onto) = walk::bound(self.program, body, base, call.written[1].0.index()) else {
            return false;
        };
        let Some((empty, _, _)) =
            super::sequence::sequence(self.program, self.program.params(call.callee)[1])
        else {
            return false;
        };
        if !self.empty_constant(onto, empty) || self.reads_after(body, base, level) != 1 {
            return false;
        }
        let mut rest = self.program.bindings(body.bindings)[level + 1 - base..].to_vec();
        rest.push(body.tail);
        rest.into_iter().any(|id| {
            let Expr::Call(callee, args) = self.program.expr(id) else {
                return false;
            };
            self.reversal(callee) && self.program.atoms(args)[0].0.index() == level
        })
    }

    /// The definition recognized as putting one sequence of type `ty` before
    /// another in order, or one written here as `reverse` reversed onto
    /// nothing and reversed again, which is recognized as one.
    fn prepending(&mut self, ty: TypeId, reverse: FnId) -> FnId {
        if let Some(&id) = self.prepends.get(&ty) {
            return id;
        }
        let found =
            self.recognised
                .sequence_over(self.program, SequenceOperation::Prepend, &[ty, ty]);
        let id = found.unwrap_or_else(|| {
            let id = super::sequence::write_prepend(self.program, ty, reverse);
            self.pure.push(true);
            id
        });
        self.prepends.insert(ty, id);
        id
    }

    /// Rewrite a body, fusing each producer read once by a consumer.
    fn rewrite_body(&mut self, body: Body, locals: Vec<Atom>, types: Vec<TypeId>) -> Body {
        let mut scope = Scope::at(locals.len());
        let (answer, _) = self.rewrite_into(body, (locals, types), &mut scope);
        copy::close(self.program, scope, answer)
    }

    /// Rewrite `body` into `scope`, each of its locals so far the atom
    /// `locals` holds and of the type `types` holds, answering what its
    /// tail comes to and its type.
    #[expect(
        clippy::too_many_lines,
        reason = "one walk rewrites calls and rebuilds arms, sharing the deferred producers"
    )]
    fn rewrite_into(
        &mut self,
        body: Body,
        (mut locals, mut types): (Vec<Atom>, Vec<TypeId>),
        scope: &mut Scope,
    ) -> (Atom, TypeId) {
        let base = locals.len();
        let bindings = self.program.bindings(body.bindings).to_vec();
        let mut deferred: HashMap<usize, Deferred> = HashMap::default();
        let count = bindings.len();
        for (at, &binding) in bindings
            .iter()
            .chain(std::iter::once(&body.tail))
            .enumerate()
        {
            let level = base + at;
            let expr = self.program.expr(binding);
            let called = match expr {
                Expr::Call(callee, args) => Some((callee, self.program.atoms(args).to_vec())),
                // Read by this match alone, which consumes it.
                Expr::Match(on, _) if deferred.contains_key(&on.0.index()) => {
                    Some(self.outlined(binding, &types))
                }
                _ => None,
            };
            let (value, ty) = if let Some((callee, written)) = called {
                let mapped: Vec<_> = written.iter().map(|a| locals[a.0.index()]).collect();
                let call = self.fusing(callee, &written, &mapped, &mut deferred);
                let value = if at < count && self.defers(&call, body, base, level) {
                    deferred.insert(level, call);
                    UNREAD
                } else {
                    let args = self.program.terms.hold_atoms(call.args.into_iter());
                    copy::push(self.program, scope, Expr::Call(call.callee, args))
                };
                (value, self.program.function(callee).result)
            } else if let Expr::Match(on, arms) = expr {
                let mut rebuilt = Vec::new();
                let mut answers = None;
                for arm in self.program.arms(arms).to_vec() {
                    let fields = self.program.fields(arm.ctor);
                    let mut inner = (locals.clone(), types.clone());
                    inner
                        .0
                        .extend((0..fields.len()).map(|at| Atom(Local::at(scope.level + at))));
                    inner.1.extend_from_slice(fields);
                    let mut arm_scope = Scope::at(scope.level + fields.len());
                    let (answer, ty) = self.rewrite_into(arm.body, inner, &mut arm_scope);
                    answers = Some(ty);
                    rebuilt.push(Arm {
                        ctor: arm.ctor,
                        body: copy::close(self.program, arm_scope, answer),
                    });
                }
                let arms = self.program.terms.hold_arms(&rebuilt);
                let value =
                    copy::push(self.program, scope, Expr::Match(locals[on.0.index()], arms));
                (value, answers.expect("a match has an arm"))
            } else {
                let ty = typed(self.program, binding, &types);
                (copy::expr(self.program, binding, &locals, scope, None), ty)
            };
            locals.push(value);
            types.push(ty);
        }
        (
            locals.pop().expect("a body answers"),
            types.pop().expect("a body answers"),
        )
    }

    /// The match `id`, taking apart the list a producer's call builds and
    /// consuming it, as a definition of its own taking the list and what
    /// its arms read from outside them, so that it is fused as any consumer
    /// is: `match xs { [] => a; [x, *rest] => rest.fold(x, f) }` walks what
    /// `xs` was made of. Its locals so far are of `types`. Answers the
    /// definition and the locals its call reads.
    fn outlined(&mut self, id: ExprId, types: &[TypeId]) -> (FnId, Vec<Atom>) {
        let Expr::Match(on, arms) = self.program.expr(id) else {
            unreachable!("only a match is outlined")
        };
        let mut read = vec![on.0.index()];
        for arm in self.program.arms(arms) {
            let operands = walk::within(self.program, arm.body)
                .flat_map(|expr| walk::operands(self.program, expr))
                .map(|atom| atom.0.index());
            read.extend(operands.filter(|&at| at < types.len()));
        }
        read[1..].sort_unstable();
        read.dedup();
        let mut map = vec![UNREAD; types.len()];
        for (at, &level) in read.iter().enumerate() {
            map[level] = Atom(Local::at(at));
        }
        let params: Vec<_> = read.iter().map(|&level| types[level]).collect();
        let name = format!(
            "{}.read",
            self.program.name(self.program.function(self.within).name)
        );
        let reader = self
            .program
            .declare(&name, &params, typed(self.program, id, types));
        self.pure.push(true);
        self.reading.push(self.within);
        let mut inner = Scope::at(params.len());
        let answer = copy::expr(self.program, id, &map, &mut inner, None);
        let body = copy::close(self.program, inner, answer);
        self.program.replace_bodies(vec![(reader, body)]);
        (
            reader,
            read.into_iter()
                .map(|level| Atom(Local::at(level)))
                .collect(),
        )
    }

    /// A call as written, or fused with the producer it reads.
    fn fusing(
        &mut self,
        callee: FnId,
        written: &[Atom],
        args: &[Atom],
        deferred: &mut HashMap<usize, Deferred>,
    ) -> Deferred {
        let plain = Deferred {
            callee,
            written: written.to_vec(),
            args: args.to_vec(),
        };
        if self.reversal(callee)
            && let Some(made) = deferred.remove(&written[0].0.index())
        {
            return self.prepended(callee, &made, written, args);
        }
        let Some(Consumer { list: taken }) = self.consumer(callee) else {
            return plain;
        };
        let Some(made) = deferred.remove(&written[taken].0.index()) else {
            return plain;
        };
        let Producer { acc, .. } = self.producer(made.callee).expect("deferred producers are");
        let fused = self.fused(made.callee, callee);
        let written_args = but(&made.written, acc).chain(but(written, taken)).collect();
        let mapped = but(&made.args, acc).chain(but(args, taken)).collect();
        // The fused walk may produce in turn, which it does where the
        // consumer did; it is asked again as it stands.
        self.producers.remove(&fused);
        self.consumers.remove(&fused);
        Deferred {
            callee: fused,
            written: written_args,
            args: mapped,
        }
    }

    /// Whether a call is a producer's, building onto nothing, whose answer
    /// is read once and by a consumer's call later in the same body.
    fn defers(&mut self, call: &Deferred, body: Body, base: usize, level: usize) -> bool {
        if self.reversal(call.callee) {
            return self.reversed_twice(call, body, base, level);
        }
        let Some(Producer { acc, .. }) = self.producer(call.callee) else {
            return false;
        };
        let Some(onto) = walk::bound(self.program, body, base, call.written[acc].0.index()) else {
            return false;
        };
        let Some((empty, _, _)) =
            super::sequence::sequence(self.program, self.program.params(call.callee)[acc])
        else {
            return false;
        };
        if !self.empty_constant(onto, empty) || self.reads_after(body, base, level) != 1 {
            return false;
        }
        // A match reading it may consume it, as a consumer's call does.
        let built = self.program.function(call.callee).result;
        let mut rest = self.program.bindings(body.bindings)[level + 1 - base..].to_vec();
        rest.push(body.tail);
        rest.into_iter()
            .enumerate()
            .any(|(at, id)| match self.program.expr(id) {
                Expr::Call(callee, args) => {
                    let args = self.program.atoms(args).to_vec();
                    self.consumer(callee)
                        .is_some_and(|Consumer { list }| args[list].0.index() == level)
                }
                Expr::Match(on, arms) => {
                    on.0.index() == level
                        && self.consumes(arms, (built, level + 1 + at), level, None)
                }
                _ => false,
            })
    }
}

/// The constructors of what a producer may walk, its end and its step: a
/// list, or a number counted down to nought, which is a list of nothing
/// but its length.
fn walked(program: &Program, ty: TypeId) -> Option<(super::CtorId, super::CtorId, usize)> {
    match super::type_shape(program, ty) {
        super::TypeShape::Natural {
            empty,
            push,
            recursive,
        }
        | super::TypeShape::Sequence {
            empty,
            push,
            recursive,
            ..
        } => Some((empty, push, recursive)),
        _ => None,
    }
}

/// The type of what `id` comes to, the locals so far of `types`.
fn typed(program: &Program, id: ExprId, types: &[TypeId]) -> TypeId {
    match program.expr(id) {
        Expr::Atom(atom) => types[atom.0.index()],
        Expr::Static(value) => program.constant(value).owner(),
        Expr::Con(ctor, _) => program.ctor(ctor).owner,
        Expr::Call(callee, _) => program.function(callee).result,
        Expr::Match(_, arms) => {
            let arm = program.arms(arms)[0];
            let mut inner = types.to_vec();
            inner.extend_from_slice(program.fields(arm.ctor));
            for &binding in program.bindings(arm.body.bindings) {
                let ty = typed(program, binding, &inner);
                inner.push(ty);
            }
            typed(program, arm.body.tail, &inner)
        }
    }
}

/// Each of `held` but the one at `skip`.
fn but<T: Copy>(held: &[T], skip: usize) -> impl Iterator<Item = T> + '_ {
    held.iter()
        .enumerate()
        .filter(move |&(at, _)| at != skip)
        .map(|(_, &item)| item)
}
