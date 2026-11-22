//! Backward liveness for inert bindings, followed by capture-free renaming.
//! A call of a definition reaching an external routine is retained even
//! when its answer is unused. A call of a pure one -- a recognized
//! operation, or a definition reaching no external routine -- is inert, and
//! so is a match whose every arm is: it answers whichever arm it takes.
//! Dropping a constructor never drops eager calls reaching external
//! routines that produced its fields.
//!
//! An inert binding is worked out only on the paths that read it: one read
//! only inside one arm of a later match moves to the top of that arm, and
//! what it reads may follow it there. Each run moves work one match down.
//! A pure binding reading a sequence that an earlier binding writes, by
//! replacing an element, adding one or joining another, is worked out
//! before the write where everything it reads is known there, so the write
//! is the sequence's last use and can be made in place: in
//! `{head: head.put(key, at), prev: prev.put(at, head.get(key))}` the read of
//! `head` goes before its `put`.
//!
//! A write replacing an element read out of the same sequence at the same
//! place, `xs.put(i, xs.get(i).ok_or([]).append(v))`, first replaces it
//! with an empty one, once nothing reads the run `xs.drop(i)` any more. The
//! element read out then has no other holder than what it was read into,
//! so where nothing else holds `xs` the element grows in place too, and
//! the empty one put there is replaced in place; where `xs` is shared, the
//! first replacement copies it and the second writes in the copy, as one
//! replacement would. Only an element that is a sequence, or wraps one, is
//! emptied, since only one of those is written in place, and only where
//! the write, after the read or first in an arm of a match on what it
//! read, is the last that reads `xs`.
//!
//! Rebuild into fresh term pools, then replace the old pools in one commit.
//! Declarations and constants stay in place; expression IDs and ranges change.

use super::terms::TermArena;
use rustc_hash::FxHashMap as HashMap;

use super::{
    Arm, Atom, Body, CtorId, Expr, ExprId, FnId, Local, Program, Range, SequenceOperation, TypeId,
    TypeShape, type_shape,
};

pub(super) fn cleanup(program: &mut Program) {
    let recognised = program.recognised().clone();
    let mut inert = Inert {
        pure: super::calls::pure(program, &recognised),
        sequences: recognised.sequences.clone(),
        known: HashMap::default(),
    };
    let mut rewrite = Rewrite {
        source: program,
        terms: TermArena::default(),
        copied: HashMap::default(),
    };
    let mut bodies = vec![];
    // Every definition is a root, not just the entry or its reachable callees.
    for at in 0..program.functions().len() {
        let id = FnId::at(at);
        let Some(body) = program.function(id).body() else {
            continue;
        };
        let body = if recognised.answered(id) {
            // Protected equations need new pool references, but no rewriting.
            rewrite.copy_body(body)
        } else {
            let count = program.params(id).len();
            let (plan, _) = plan_body(program, &mut inert, body, count);
            let locals = (0..count).map(|at| Some(Atom(Local::at(at)))).collect();
            rewrite.body(plan, locals, count)
        };
        bodies.push((id, body));
    }
    program.replace_terms(rewrite.terms, bodies);
}

// Plans belong to occurrences, not ExprIds: an arena expression can be used
// in different lexical scopes. Only retained matches have child body plans.
struct PlannedBody {
    /// Bindings of an enclosing body moved to the top of this one, each
    /// with the local it bound there, last first.
    sunk: Vec<(usize, PlannedExpr)>,
    bindings: Vec<Option<PlannedExpr>>,
    /// The order the kept bindings are worked out in, by index.
    order: Vec<usize>,
    /// Elements emptied before they are written: see [`Order::emptied`].
    emptied: Vec<Emptied>,
    tail: PlannedExpr,
}

/// An element of a sequence replaced with an empty one after the binding
/// `after`, or first where there is none, before a later write replaces
/// it: the sequence's and the place's locals, and the constructors building
/// the empty element, outermost first.
struct Emptied {
    after: Option<usize>,
    sequence: Atom,
    place: Atom,
    replace: FnId,
    empty: Vec<CtorId>,
}

struct PlannedExpr {
    id: ExprId,
    arms: Vec<PlannedBody>,
}

/// The elements a body empties: see [`Order::planned`].
type Emptying = Vec<(Option<(usize, usize)>, Emptied)>;

/// Where a body's local is read, as far as that body can tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Read {
    Unread,
    /// Only inside one arm of the match the body binds or answers at `site`:
    /// a binding's index, or the number of bindings for the tail.
    InArm {
        site: usize,
        arm: usize,
    },
    /// Anywhere else, or in more than one arm.
    Here,
}

impl Read {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Unread, read) | (read, Self::Unread) => read,
            (read, other) if read == other => read,
            _ => Self::Here,
        }
    }
}

/// Which expressions may go unevaluated where nothing reads them, found
/// once for each: an expression is inert wherever it stands.
struct Inert {
    /// Indexed by definition: see [`super::calls::pure`].
    pure: Vec<bool>,
    /// Indexed by definition: the sequence operation it is recognized as.
    sequences: Vec<Option<SequenceOperation>>,
    known: HashMap<ExprId, bool>,
}

impl Inert {
    /// Whether `called` writes a sequence it is handed, which lowering does
    /// in place where nothing else holds it.
    fn writes(&self, called: FnId) -> bool {
        matches!(
            self.sequences[called.index()],
            Some(
                SequenceOperation::Replace
                    | SequenceOperation::Append
                    | SequenceOperation::Prepend
                    | SequenceOperation::ReverseOnto
            )
        )
    }

    /// Whether `called` answers a run of the sequence it is handed, which
    /// lowering holds in the same buffer.
    fn views(&self, called: FnId) -> bool {
        matches!(
            self.sequences[called.index()],
            Some(SequenceOperation::Drop | SequenceOperation::Take)
        )
    }

    fn is(&mut self, program: &Program, id: ExprId) -> bool {
        if let Some(&known) = self.known.get(&id) {
            return known;
        }
        let inert = match program.expr(id) {
            Expr::Call(called, _) => self.pure[called.index()],
            Expr::Match(_, arms) => program.arms(arms).iter().all(|arm| {
                program
                    .bindings(arm.body.bindings)
                    .iter()
                    .chain([&arm.body.tail])
                    .all(|&inner| self.is(program, inner))
            }),
            Expr::Atom(..) | Expr::Static(..) | Expr::Con(..) => true,
        };
        self.known.insert(id, inert);
        inert
    }
}

/// Plan `body`, whose first `base` locals are its enclosing scope's, and
/// say which of those it reads.
fn plan_body(
    program: &Program,
    inert: &mut Inert,
    body: Body,
    base: usize,
) -> (PlannedBody, Vec<bool>) {
    let bindings = program.bindings(body.bindings);
    let mut reads = vec![Read::Unread; base + bindings.len()];
    let mut tail = plan_expr(program, inert, body.tail, &mut reads, bindings.len(), None);
    let mut kept: Vec<Option<PlannedExpr>> = (0..bindings.len()).map(|_| None).collect();
    // Where each binding is worked out: its own index, the site it is
    // moved into, or nowhere.
    let mut sites = vec![None; bindings.len()];
    for (index, &id) in bindings.iter().enumerate().rev() {
        let read = reads.pop().expect("one local per binding");
        match (read, inert.is(program, id)) {
            (Read::Unread, true) => {}
            (Read::InArm { site, arm }, true) => {
                sites[index] = Some(site);
                let planned = plan_expr(program, inert, id, &mut reads, index, Some((site, arm)));
                let target = if site == bindings.len() {
                    &mut tail
                } else {
                    kept[site]
                        .as_mut()
                        .expect("a binding read in an arm is kept")
                };
                target.arms[arm].sunk.push((base + index, planned));
            }
            _ => {
                sites[index] = Some(index);
                kept[index] = Some(plan_expr(program, inert, id, &mut reads, index, None));
            }
        }
    }
    let captures = reads.iter().map(|&read| read != Read::Unread).collect();
    let (order, emptying) = Order::new(program, body, base, sites).planned(program, inert);
    let planned = PlannedBody {
        sunk: vec![],
        bindings: kept,
        order,
        emptied: vec![],
        tail,
    };
    (planned.emptying(emptying), captures)
}

impl PlannedBody {
    /// Each element in `emptying` emptied where it is: in this body, or
    /// first in an arm of one of its matches.
    fn emptying(mut self, emptying: Emptying) -> Self {
        for (within, empty) in emptying {
            let Some((site, arm)) = within else {
                self.emptied.push(empty);
                continue;
            };
            let target = self.bindings.get_mut(site).map_or(&mut self.tail, |kept| {
                kept.as_mut().expect("a write's match is kept")
            });
            target.arms[arm].emptied.push(empty);
        }
        self
    }
}

/// Plan the expression at `site` of the body `reads` belongs to, which
/// stands there or, where `within` says, at the top of an arm of a later
/// match, where every local it reads is read too.
fn plan_expr(
    program: &Program,
    inert: &mut Inert,
    id: ExprId,
    reads: &mut [Read],
    site: usize,
    within: Option<(usize, usize)>,
) -> PlannedExpr {
    let here = within.map_or(Read::Here, |(site, arm)| Read::InArm { site, arm });
    let read = |reads: &mut [Read], a: Atom, as_read: Read| {
        let at = &mut reads[a.0.index()];
        *at = at.and(as_read);
    };
    let mut children = vec![];
    match program.expr(id) {
        Expr::Atom(a) => read(reads, a, here),
        Expr::Static(..) => {}
        Expr::Con(_, args) | Expr::Call(_, args) => {
            for &a in program.atoms(args) {
                read(reads, a, here);
            }
        }
        Expr::Match(value, arms) => {
            read(reads, value, here);
            for (index, arm) in program.arms(arms).iter().enumerate() {
                let in_arm = within.map_or(Read::InArm { site, arm: index }, |_| here);
                let base = reads.len() + program.fields(arm.ctor).len();
                let (plan, captures) = plan_body(program, inert, arm.body, base);
                // Fields are local to this arm, not captures of its parent.
                for (at, captured) in captures.into_iter().enumerate().take(reads.len()) {
                    if captured {
                        reads[at] = reads[at].and(in_arm);
                    }
                }
                children.push(plan);
            }
        }
    }
    PlannedExpr { id, arms: children }
}

/// Where a body's bindings are worked out and what each reads, for moving
/// reads of a sequence before a write of it.
struct Order {
    bindings: Vec<ExprId>,
    tail: ExprId,
    base: usize,
    /// Where each binding is worked out: see [`plan_body`].
    sites: Vec<Option<usize>>,
    /// The locals each kept binding reads, with those of what is moved into
    /// it, by index.
    reads: Vec<Vec<usize>>,
    /// The locals the body's answer reads, with those of what is moved into
    /// it.
    answer: Vec<usize>,
}

impl Order {
    fn new(program: &Program, body: Body, base: usize, sites: Vec<Option<usize>>) -> Self {
        let bindings = program.bindings(body.bindings).to_vec();
        let mut reads = vec![vec![]; bindings.len()];
        let mut answer = vec![];
        free_reads(program, body.tail, base + bindings.len(), &mut answer);
        for (index, &id) in bindings.iter().enumerate() {
            match sites[index] {
                Some(site) if site < bindings.len() => {
                    free_reads(program, id, base + index, &mut reads[site]);
                }
                Some(_) => free_reads(program, id, base + index, &mut answer),
                None => {}
            }
        }
        Self {
            bindings,
            tail: body.tail,
            base,
            sites,
            reads,
            answer,
        }
    }

    /// The kept bindings' order: as written, but where a binding writes a
    /// sequence that only later pure bindings read again, and what they read
    /// besides is known before the write, those bindings, and the ones they
    /// read in between, are worked out just before it.
    fn reads_first(&self, program: &Program, inert: &mut Inert) -> Vec<usize> {
        let kept: Vec<usize> = (0..self.bindings.len())
            .filter(|&index| self.sites[index] == Some(index))
            .collect();
        let mut order = kept.clone();
        for &write in &kept {
            let Expr::Call(called, args) = program.expr(self.bindings[write]) else {
                continue;
            };
            if !inert.writes(called) {
                continue;
            }
            for &Atom(written) in program.atoms(args) {
                let written = written.index();
                if self.answer.contains(&written) {
                    continue;
                }
                if let Some(moved) = self.moved(program, inert, &order, (write, written)) {
                    let at = order
                        .iter()
                        .position(|&index| index == write)
                        .expect("kept");
                    let rest = order[at..].iter().filter(|index| !moved.contains(index));
                    let mut reordered = order[..at].to_vec();
                    reordered.extend(order[at..].iter().filter(|index| moved.contains(index)));
                    reordered.extend(rest);
                    order = reordered;
                }
            }
        }
        order
    }

    /// The bindings after `write` in `order` that read the local `written`
    /// or a run of it, with the ones after it they read, where every one is
    /// pure, writes none of those, and none reads `write`'s answer.
    fn moved(
        &self,
        program: &Program,
        inert: &mut Inert,
        order: &[usize],
        (write, written): (usize, usize),
    ) -> Option<Vec<usize>> {
        let at = order.iter().position(|&index| index == write)?;
        // The sequence, and the runs of it moved bindings answer, which
        // hold its buffer too and so must end before the write as well.
        let mut held = vec![written];
        let mut moved: Vec<usize> = vec![];
        let mut next = 0;
        loop {
            for &index in &order[at + 1..] {
                if !moved.contains(&index) && self.reads[index].iter().any(|r| held.contains(r)) {
                    moved.push(index);
                }
            }
            let Some(&index) = moved.get(next) else {
                break;
            };
            next += 1;
            let id = self.bindings[index];
            if !inert.is(program, id) || writes(program, inert, id, &held) {
                return None;
            }
            if let Expr::Call(called, args) = program.expr(id)
                && inert.views(called)
                && program
                    .atoms(args)
                    .first()
                    .is_some_and(|a| held.contains(&a.0.index()))
            {
                held.push(self.base + index);
                if self.answer.contains(&(self.base + index)) {
                    return None;
                }
            }
            self.needed(order, (at, index), &mut moved)?;
        }
        (!moved.is_empty()).then_some(moved)
    }

    /// The kept bindings' order, and the elements emptied in it: each in
    /// this body, or first in an arm of a match, by the match's binding, or
    /// the tail as the number of bindings, and the arm.
    fn planned(&self, program: &Program, inert: &mut Inert) -> (Vec<usize>, Emptying) {
        let order = self.reads_first(program, inert);
        let tail = self.bindings.len();
        let mut emptied = vec![];
        for (at, write) in order.iter().copied().chain([tail]).enumerate() {
            let id = self.bindings.get(write).copied().unwrap_or(self.tail);
            if let Expr::Match(_, arms) = program.expr(id) {
                for (arm, &Arm { body, .. }) in program.arms(arms).iter().enumerate() {
                    let ids = program.bindings(body.bindings).iter().chain([&body.tail]);
                    for &id in ids {
                        let empty =
                            self.emptied(program, inert, &order, ((at, write), (id, Some(body))));
                        emptied.extend(empty.map(|empty| (Some((write, arm)), empty)));
                    }
                }
            }
            let empty = self.emptied(program, inert, &order, ((at, write), (id, None)));
            emptied.extend(empty.map(|empty| (None, empty)));
        }
        (order, emptied)
    }

    /// Where `write`, at `at` in `order`, is `id`, `put(xs, i, e)`, or a
    /// match one of whose arms, `arm`, holds `id`, and an earlier binding
    /// `xs.drop(i)` read the element: the element emptied after the last
    /// binding reading that run, or a run of it, or first in the arm, where
    /// nothing from there on reads `xs` but `id`.
    fn emptied(
        &self,
        program: &Program,
        inert: &Inert,
        order: &[usize],
        ((at, write), (id, arm)): ((usize, usize), (ExprId, Option<Body>)),
    ) -> Option<Emptied> {
        let call = |id: ExprId, operation| match program.expr(id) {
            Expr::Call(called, args) if inert.sequences[called.index()] == Some(operation) => {
                Some((called, program.atoms(args)))
            }
            _ => None,
        };
        let (replace, &[sequence, place, _]) = call(id, SequenceOperation::Replace)? else {
            return None;
        };
        let empty = empty_element(program, program.params(replace)[0])?;
        let read = order[..at].iter().rposition(|&index| {
            call(self.bindings[index], SequenceOperation::Drop)
                .is_some_and(|(_, args)| args == [sequence, place])
        })?;
        let (after, held) = self.run_read(program, inert, order, (read, at));
        let local = sequence.0.index();
        let tail = self.bindings.len();
        let mut later = order[after + 1..].iter().copied().chain([tail]);
        let mut others = later.any(|index| index != write && self.reads(index).contains(&local));
        others |= self.reads(write).iter().any(|r| held.contains(r));
        // In an arm, nothing else in it reads `xs`.
        let mut beside = vec![];
        if let Some(body) = arm {
            for &other in program.bindings(body.bindings).iter().chain([&body.tail]) {
                if other != id {
                    free_reads(program, other, local + 1, &mut beside);
                }
            }
        }
        (!others && !beside.contains(&local)).then_some(Emptied {
            after: arm.is_none().then(|| order[after]),
            sequence,
            place,
            replace,
            empty,
        })
    }

    /// Where in `order`, before `at`, the run the binding at `read` answers,
    /// or a run of it, is last read, and the locals holding those runs.
    fn run_read(
        &self,
        program: &Program,
        inert: &Inert,
        order: &[usize],
        (read, at): (usize, usize),
    ) -> (usize, Vec<usize>) {
        let mut held = vec![self.base + order[read]];
        let mut after = read;
        for (between, &index) in order.iter().enumerate().take(at).skip(read + 1) {
            if self.reads(index).iter().any(|r| held.contains(r)) {
                after = between;
                if let Expr::Call(called, _) = program.expr(self.bindings[index])
                    && inert.views(called)
                {
                    held.push(self.base + index);
                }
            }
        }
        (after, held)
    }

    /// The locals the binding `index` reads, or the answer where it is the
    /// number of bindings.
    fn reads(&self, index: usize) -> &[usize] {
        self.reads.get(index).unwrap_or(&self.answer)
    }

    /// Add to `moved` the bindings after the write at `at` in `order` that
    /// the binding `index` reads, or `None` where it reads the write's
    /// answer.
    fn needed(
        &self,
        order: &[usize],
        (at, index): (usize, usize),
        moved: &mut Vec<usize>,
    ) -> Option<()> {
        for &read in &self.reads[index] {
            let Some(site) = read.checked_sub(self.base).and_then(|at| self.sites[at]) else {
                continue;
            };
            if site == index || moved.contains(&site) {
                continue;
            }
            let place = order.iter().position(|&other| other == site)?;
            if place == at {
                return None;
            }
            if place > at {
                moved.push(site);
            }
        }
        Some(())
    }
}

/// The constructors building an empty element of the sequence type `list`,
/// outermost first, where it is a sequence or wraps one.
fn empty_element(program: &Program, list: TypeId) -> Option<Vec<CtorId>> {
    let TypeShape::Sequence { mut element, .. } = type_shape(program, list) else {
        return None;
    };
    let mut empty = vec![];
    loop {
        match type_shape(program, element) {
            TypeShape::Sequence { empty: nothing, .. } => {
                empty.push(nothing);
                return Some(empty);
            }
            TypeShape::Wrapper { constructor, field } => {
                empty.push(constructor);
                element = field;
            }
            _ => return None,
        }
    }
}

/// Add to `out` each local below `limit` that `id` reads, inside the arms
/// of a match too.
fn free_reads(program: &Program, id: ExprId, limit: usize, out: &mut Vec<usize>) {
    let mut read = |Atom(local): Atom| {
        if local.index() < limit && !out.contains(&local.index()) {
            out.push(local.index());
        }
    };
    match program.expr(id) {
        Expr::Atom(a) => read(a),
        Expr::Static(..) => {}
        Expr::Con(_, args) | Expr::Call(_, args) => {
            program.atoms(args).iter().for_each(|&a| read(a));
        }
        Expr::Match(value, arms) => {
            read(value);
            for arm in program.arms(arms) {
                let body = arm.body;
                for &inner in program.bindings(body.bindings).iter().chain([&body.tail]) {
                    free_reads(program, inner, limit, out);
                }
            }
        }
    }
}

/// Whether `id` writes one of the locals `written` as a sequence, inside the
/// arms of a match too.
fn writes(program: &Program, inert: &Inert, id: ExprId, written: &[usize]) -> bool {
    match program.expr(id) {
        Expr::Call(called, args) => {
            inert.writes(called)
                && program
                    .atoms(args)
                    .iter()
                    .any(|a| written.contains(&a.0.index()))
        }
        Expr::Match(_, arms) => program.arms(arms).iter().any(|arm| {
            let body = arm.body;
            program
                .bindings(body.bindings)
                .iter()
                .chain([&body.tail])
                .any(|&inner| writes(program, inert, inner, written))
        }),
        Expr::Atom(..) | Expr::Static(..) | Expr::Con(..) => false,
    }
}

struct Rewrite<'a> {
    source: &'a Program,
    terms: TermArena,
    // Copying protected equations is context-free, unlike liveness rewriting.
    // Preserve expression sharing there, including shared match subtrees.
    copied: HashMap<ExprId, ExprId>,
}

impl Rewrite<'_> {
    fn body(
        &mut self,
        mut plan: PlannedBody,
        mut locals: Vec<Option<Atom>>,
        mut level: usize,
    ) -> Body {
        let mut bindings = vec![];
        // Moved here from an enclosing body, they come first, in the order
        // they were written, each read in the scope it was written in: the
        // locals before its own, which a match's fields are numbered after.
        for (local, sunk) in plan.sunk.into_iter().rev() {
            let value = if let Expr::Atom(a) = self.source.expr(sunk.id) {
                self::local(&locals, a)
            } else {
                let expr = self.expr(sunk, &locals[..local], level);
                bindings.push(self.terms.push(expr));
                level += 1;
                Atom(Local::at(level - 1))
            };
            locals[local] = Some(value);
        }
        level = self.empty(&plan.emptied, None, &mut locals, (&mut bindings, level));
        // Each binding reads the locals written before it, which the order
        // has worked out first, and a match's fields are numbered after them.
        let start = locals.len();
        locals.resize(start + plan.bindings.len(), None);
        for index in plan.order {
            let binding = plan.bindings[index]
                .take()
                .expect("an ordered binding is kept");
            let value = if let Expr::Atom(a) = self.source.expr(binding.id) {
                local(&locals, a)
            } else {
                let expr = self.expr(binding, &locals[..start + index], level);
                bindings.push(self.terms.push(expr));
                level += 1;
                Atom(Local::at(level - 1))
            };
            locals[start + index] = Some(value);
            let after = Some(index);
            level = self.empty(&plan.emptied, after, &mut locals, (&mut bindings, level));
        }
        let expr = self.expr(plan.tail, &locals, level);
        // A body answering its last binding is that binding's expression.
        let tail = match expr {
            Expr::Atom(answer) if answer.0.index() + 1 == level && !bindings.is_empty() => {
                bindings.pop().expect("the last binding")
            }
            _ => self.terms.push(expr),
        };
        self.terms.hold_body(&bindings, tail)
    }

    /// Bind each element emptied `after` a binding, or first, and the
    /// sequence with it put in place of the one read out, which the write,
    /// the only one reading the sequence from here on, then reads in its
    /// place. The level after them.
    fn empty(
        &mut self,
        emptied: &[Emptied],
        after: Option<usize>,
        locals: &mut [Option<Atom>],
        (bindings, mut level): (&mut Vec<ExprId>, usize),
    ) -> usize {
        for empty in emptied.iter().filter(|empty| empty.after == after) {
            let mut element = None;
            for &ctor in empty.empty.iter().rev() {
                let args = self.terms.hold_atoms(element.into_iter());
                bindings.push(self.terms.push(Expr::Con(ctor, args)));
                element = Some(Atom(Local::at(level)));
                level += 1;
            }
            let mut args = vec![local(locals, empty.sequence), local(locals, empty.place)];
            args.extend(element);
            let args = self.terms.hold_atoms(args.into_iter());
            bindings.push(self.terms.push(Expr::Call(empty.replace, args)));
            locals[empty.sequence.0.index()] = Some(Atom(Local::at(level)));
            level += 1;
        }
        level
    }

    fn expr(&mut self, plan: PlannedExpr, locals: &[Option<Atom>], level: usize) -> Expr {
        match self.source.expr(plan.id) {
            Expr::Atom(a) => Expr::Atom(local(locals, a)),
            value @ Expr::Static(..) => value,
            Expr::Con(ctor, args) => Expr::Con(ctor, self.args(args, locals)),
            Expr::Call(called, args) => Expr::Call(called, self.args(args, locals)),
            Expr::Match(value, arms) => {
                let mut found = vec![];
                for (arm, body) in self.source.arms(arms).iter().zip(plan.arms) {
                    let fields = self.source.fields(arm.ctor).len();
                    let mut inner = locals.to_vec();
                    inner.extend((0..fields).map(|at| Some(Atom(Local::at(level + at)))));
                    found.push(Arm {
                        ctor: arm.ctor,
                        body: self.body(body, inner, level + fields),
                    });
                }
                Expr::Match(local(locals, value), self.terms.hold_arms(&found))
            }
        }
    }

    fn args(&mut self, args: Range, locals: &[Option<Atom>]) -> Range {
        self.terms
            .hold_atoms(self.source.atoms(args).iter().map(|&a| local(locals, a)))
    }

    fn copy_body(&mut self, body: Body) -> Body {
        let bindings: Vec<_> = self
            .source
            .bindings(body.bindings)
            .iter()
            .map(|&id| self.copy_expr(id))
            .collect();
        let tail = self.copy_expr(body.tail);
        self.terms.hold_body(&bindings, tail)
    }

    fn copy_expr(&mut self, id: ExprId) -> ExprId {
        if let Some(&copied) = self.copied.get(&id) {
            return copied;
        }
        let expr = match self.source.expr(id) {
            value @ (Expr::Atom(_) | Expr::Static(..)) => value,
            Expr::Con(ctor, args) => Expr::Con(ctor, self.copy_args(args)),
            Expr::Call(called, args) => Expr::Call(called, self.copy_args(args)),
            Expr::Match(value, arms) => {
                let arms: Vec<_> = self
                    .source
                    .arms(arms)
                    .iter()
                    .map(|arm| Arm {
                        ctor: arm.ctor,
                        body: self.copy_body(arm.body),
                    })
                    .collect();
                Expr::Match(value, self.terms.hold_arms(&arms))
            }
        };
        let copied = self.terms.push(expr);
        self.copied.insert(id, copied);
        copied
    }

    fn copy_args(&mut self, args: Range) -> Range {
        self.terms
            .hold_atoms(self.source.atoms(args).iter().copied())
    }
}

fn local(locals: &[Option<Atom>], a: Atom) -> Atom {
    locals[a.0.index()].expect("a retained expression reads only live locals")
}
