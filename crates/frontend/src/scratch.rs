//! A directory of source files a test writes, removed when dropped, and
//! the small standard library the frontend's own tests load beside it.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// How many directories this process has made, so two made in the same
/// nanosecond differ.
static MADE: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Scratch(pub(crate) PathBuf);

impl Scratch {
    /// A directory of its own, named after `name`: the process and a count
    /// keep tests run side by side, as nextest runs them, from sharing one
    /// and removing it under each other.
    pub(crate) fn new(name: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let process = std::process::id();
        let made = MADE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("faun-{name}-{process}-{made}-{nanos}"));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    /// `source` written at `name`, and where.
    pub(crate) fn write(&self, name: &str, source: &str) -> PathBuf {
        let path = self.0.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, source).unwrap();
        path
    }

    /// A standard library of `Bool`, `Order`, `Nat`, `Int`, `U8`, `List`,
    /// `Result` and `Str`, each with only the methods literals, patterns
    /// and comparisons need: its root.
    pub(crate) fn standard(&self) -> PathBuf {
        self.write("bool.fn", "export {Bool.*}\nBool : [False, True].(not : Bool -> Bool\nnot(False) -> True\nnot(True) -> False\nand : (Bool, Bool) -> Bool\nand(True, True) -> True\nand(_, _) -> False\nor : (Bool, Bool) -> Bool\nor(False, False) -> False\nor(_, _) -> True)\n");
        self.write("nat.fn", "export {Nat.*}\nNat : [Zero, Succ(Nat)].(add : (Nat, Nat) -> Nat\nadd(0, y) -> y\nadd(Succ(n), y) -> Succ(n.add(y)))\n");
        self.write(
            "int.fn",
            "export {Int.*}\nInt : [Pos(Nat), NegSucc(Nat)].(of_nat : Nat -> Int\nof_nat(n) -> Pos(n)\nneg : Int -> Int\nneg(i) -> i)\n",
        );
        self.write("u8.fn", "export {U8}\nU8 : Nat\n");
        self.write(
            "list.fn",
            "export {List.*}\nList(a) : [Nil, Cons(a, List(a))].(len : List(a) -> Nat\nlen(_) -> 0\ntake : (List(a), Nat) -> List(a)\ntake(values, _) -> values\ndrop : (List(a), Nat) -> List(a)\ndrop(values, _) -> values)\n",
        );
        self.write(
            "str.fn",
            "export {Str}\nStr : List(U8).(concat : (Str, Str) -> Str\nconcat(left, _) -> left\nequals : (Str, Str) -> Bool\nequals(_, _) -> False)\n",
        );
        self.write(
            "result.fn",
            "export {Result.*}\nResult(a, e) : [Ok(a), Err(e)]\n",
        );
        self.write(
            "order.fn",
            "export {Order.*}\nOrder : [Lt, Eq, Gt].(lt : Order -> Bool\nlt(Lt) -> True\nlt(_) -> False\nle : Order -> Bool\nle(Gt) -> False\nle(_) -> True\ngt : Order -> Bool\ngt(Gt) -> True\ngt(_) -> False\nge : Order -> Bool\nge(Lt) -> False\nge(_) -> True\neq : Order -> Bool\neq(Eq) -> True\neq(_) -> False\nor : (Order, Order) -> Order\nor(Eq, next) -> next\nor(decided, _) -> decided)\n",
        );
        self.write(
            "standard.fn",
            "package {}\nimport bool {Bool.*}\nimport order {Order.*}\nimport nat {Nat.*}\nimport int {Int.*}\nimport u8 {U8}\nimport list {List.*}\nimport result {Result.*}\nimport str {Str}\nexport {Bool.*, Order.*, Nat.*, Int.*, U8, List.*, Result.*, Str}\n",
        )
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
