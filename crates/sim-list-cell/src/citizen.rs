//! Citizen descriptor for the cons-cell list class, mapping list items to and
//! from their serialized expression form via [`cons_list_class_symbol`] and
//! [`ConsListDescriptor`].

use sim_citizen_derive::Citizen;
use sim_kernel::{Error, Expr, Result, Symbol};

/// Serialized citizen form of a [`crate::ConsList`]: the list's items as a
/// vector of [`Expr`], encoded and decoded under the `list/ConsList` class.
#[derive(Clone, Debug, Default, PartialEq, Citizen)]
#[citizen(symbol = "list/ConsList", version = 0)]
pub struct ConsListDescriptor {
    /// The list elements in order, each as a serialized expression.
    #[citizen(with = "crate::citizen::expr_items")]
    pub items: Vec<Expr>,
}

/// The class symbol for the cons-cell list class, `list/ConsList`.
pub fn cons_list_class_symbol() -> Symbol {
    Symbol::qualified("list", "ConsList")
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
