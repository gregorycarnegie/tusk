//! SQL Server's dialect for sea-query, which ships only MySQL, Postgres and
//! SQLite. Net2's `customquery/querydb` takes one finished SQL string, with no
//! parameters to bind, so every value is written into the text, and how it is
//! quoted is what keeps it a value:
//!
//! - names in `[brackets]`, a `]` inside one doubled;
//! - text as `N'…'`, a `'` inside doubled. Backslashes mean nothing in T-SQL,
//!   unlike MySQL's escapes. Without the `N`, letters outside the server's code
//!   page (Łukasz) turn into `?` and stop matching;
//! - booleans as `1`/`0`, as T-SQL has no TRUE or FALSE.
//!
//! LIMIT and OFFSET are not translated (T-SQL wants TOP); a query that sets
//! them fails in Net2 rather than here.
use std::fmt::{self, Write};

use sea_query::{
    BinOper, EscapeBuilder, ExplainStatement, Expr, Oper, OperLeftAssocDecider, PrecedenceDecider,
    QueryBuilder, Quote, QuotedBuilder, SelectInto, SelectStatement, SqlWriter, SubQueryStatement,
    TableRefBuilder, Value,
};

#[derive(Clone, Copy, Default)]
pub struct TSql;

/// The statement as text for querydb.
pub fn sql(statement: &SelectStatement) -> String {
    statement.to_string(TSql)
}

impl QuotedBuilder for TSql {
    fn quote(&self) -> Quote {
        ('[', ']').into()
    }
}

impl EscapeBuilder for TSql {
    fn write_escaped(&self, buffer: &mut impl Write, string: &str) {
        for c in string.chars() {
            match c {
                '\'' => buffer.write_str("''"),
                c => buffer.write_char(c),
            }
            .unwrap()
        }
    }

    fn unescape_string(&self, string: &str) -> String {
        string.replace("''", "'")
    }
}

impl TableRefBuilder for TSql {}

// sea-query keeps its shared precedence rules private. A single item never
// needs parentheses; anything compound always keeps them, which is never wrong.
impl PrecedenceDecider for TSql {
    fn inner_expr_well_known_greater_precedence(&self, inner: &Expr, _: &Oper) -> bool {
        matches!(
            inner,
            Expr::Column(_)
                | Expr::Tuple(_)
                | Expr::Constant(_)
                | Expr::FunctionCall(_)
                | Expr::Value(_)
                | Expr::Keyword(_)
                | Expr::Case(_)
                | Expr::SubQuery(..)
                | Expr::TypeName(_)
        )
    }
}

impl OperLeftAssocDecider for TSql {
    fn well_known_left_associative(&self, _: &BinOper) -> bool {
        false
    }
}

impl QueryBuilder for TSql {
    fn prepare_query_statement(&self, query: &SubQueryStatement, sql: &mut impl SqlWriter) {
        match query {
            SubQueryStatement::SelectStatement(s) => self.prepare_select_statement(s, sql),
            SubQueryStatement::InsertStatement(s) => self.prepare_insert_statement(s, sql),
            SubQueryStatement::UpdateStatement(s) => self.prepare_update_statement(s, sql),
            SubQueryStatement::DeleteStatement(s) => self.prepare_delete_statement(s, sql),
            SubQueryStatement::WithStatement(s) => self.prepare_with_query(s, sql),
        }
    }

    // Tusk only reads, and T-SQL spells neither of these sea-query's way.
    fn prepare_select_into(&self, _: &SelectInto, _: &mut impl SqlWriter) {}
    fn prepare_explain_statement(&self, _: &ExplainStatement, _: &mut impl SqlWriter) {}

    fn prepare_value(&self, value: Value, sql: &mut impl SqlWriter) {
        sql.push_param(value, self as _);
    }

    fn write_string_quoted(&self, string: &str, buffer: &mut impl Write) {
        buffer.write_str("N'").unwrap();
        self.write_escaped(buffer, string);
        buffer.write_str("'").unwrap();
    }

    fn write_value(&self, buffer: &mut impl Write, value: &Value) -> fmt::Result {
        match value {
            Value::Bool(Some(b)) => buffer.write_str(if *b { "1" } else { "0" }),
            other => self.write_value_common(buffer, other),
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use sea_query::{ExprTrait, JoinType, Order, Query};

    use super::*;

    fn wh(condition: Expr) -> String {
        let mut query = Query::select();
        query
            .column(("u", "UserID"))
            .from_as(("sdk", "UsersEx"), "u")
            .and_where(condition);
        sql(&query)
    }

    #[test]
    fn names_and_numbers_are_bracketed_and_bare() {
        let mut query = Query::select();
        query
            .expr_as(Expr::col(("c", "LostCard")), "lost")
            .from_as(("sdk", "Cards"), "c")
            .join_as(
                JoinType::LeftJoin,
                ("sdk", "UsersEx"),
                "u",
                Expr::col(("u", "UserID")).equals(("c", "UserID")),
            )
            .and_where(Expr::col(("c", "CardNumber")).eq(34935097u32))
            .order_by(("c", "LostCard"), Order::Asc);
        assert_eq!(
            sql(&query),
            "SELECT [c].[LostCard] AS [lost] FROM [sdk].[Cards] AS [c] \
             LEFT JOIN [sdk].[UsersEx] AS [u] ON [u].[UserID] = [c].[UserID] \
             WHERE [c].[CardNumber] = 34935097 ORDER BY [c].[LostCard] ASC"
        );
    }

    /// Whatever the text, it stays one N'…' value: a quote cannot end it, and
    /// nothing else in T-SQL can either.
    #[test]
    fn text_cannot_break_out_of_its_quotes() {
        let field = |text: &str| wh(Expr::col(("u", "Field14_50")).eq(text));
        assert!(field("O'Brien").ends_with("WHERE [u].[Field14_50] = N'O''Brien'"));
        assert!(field("x' OR 1=1 --").ends_with("WHERE [u].[Field14_50] = N'x'' OR 1=1 --'"));
        assert!(
            field("a\\'b").ends_with("= N'a\\''b'"),
            "a backslash is just a backslash"
        );
        assert!(field("Łukasz Żółć").ends_with("= N'Łukasz Żółć'"));
        assert!(field("two\nlines").ends_with("= N'two\nlines'"));
        assert!(field("").ends_with("= N''"));
    }

    #[test]
    fn a_bracket_in_a_name_is_doubled_and_booleans_are_bits() {
        assert!(wh(Expr::col(("u", "Odd]Name")).eq(1)).ends_with("WHERE [u].[Odd]]Name] = 1"));
        assert!(wh(Expr::col(("u", "Active")).eq(true)).ends_with("WHERE [u].[Active] = 1"));
        assert!(wh(Expr::col(("u", "Active")).eq(false)).ends_with("WHERE [u].[Active] = 0"));
    }

    use proptest::prelude::*;

    proptest! {
        /// Read back the way SQL Server reads it, any text is the value it
        /// started as, and the literal ends exactly where the SQL does.
        #[test]
        fn any_text_reads_back_as_itself(text in r"(\PC|['\]\[\\\n-])*") {
            let query = wh(Expr::col(("u", "Field14_50")).eq(text.as_str()));
            let literal = query.split_once("= N'").unwrap().1;
            let body = literal.strip_suffix('\'').unwrap();
            // Inside, every quote comes in pairs: none can close the literal early.
            prop_assert!(!body.replace("''", "").contains('\''));
            prop_assert_eq!(body.replace("''", "'"), text);
        }
    }
}
