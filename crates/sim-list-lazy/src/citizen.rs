//! Citizen descriptors for the lazy list classes, mapping forced list items to
//! and from their serialized expression form via the class-symbol accessors
//! and the [`LazyConsListDescriptor`]/[`LazyIterListDescriptor`] records.

use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

/// Serialized citizen form of a [`crate::LazyConsList`]: its forced items as a
/// vector of [`Expr`], encoded and decoded under the `list/LazyConsList` class.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "list/LazyConsList", version = 0)]
pub struct LazyConsListDescriptor {
    /// The forced list elements in order, each as a serialized expression.
    #[citizen(with = "crate::citizen::expr_items")]
    pub items: Vec<Expr>,
}

/// Serialized citizen form of a [`crate::LazyIterList`]: its forced items as a
/// vector of [`Expr`], encoded and decoded under the `list/LazyIterList` class.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "list/LazyIterList", version = 0)]
pub struct LazyIterListDescriptor {
    /// The forced list elements in order, each as a serialized expression.
    #[citizen(with = "crate::citizen::expr_items")]
    pub items: Vec<Expr>,
}

/// The class symbol for the lazy cons list class, `list/LazyConsList`.
pub fn lazy_cons_list_class_symbol() -> Symbol {
    Symbol::qualified("list", "LazyConsList")
}

/// The class symbol for the lazy iterator list class, `list/LazyIterList`.
pub fn lazy_iter_list_class_symbol() -> Symbol {
    Symbol::qualified("list", "LazyIterList")
}

pub(crate) mod expr_items {
    use super::*;

    pub fn encode(items: &[Expr]) -> Expr {
        Expr::List(items.to_vec())
    }

    pub fn decode(expr: &Expr) -> Result<Vec<Expr>> {
        let Expr::List(items) = expr else {
            return Err(Error::Eval(
                "list citizen field items: expected list".to_owned(),
            ));
        };
        Ok(items.clone())
    }
}
