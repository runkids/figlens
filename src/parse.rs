//! Bounded preflight of the pinned Kiwi decoder's schema and value paths.

use crate::FigError;
use kiwi_schema::{
    ByteBuffer, DefKind, Field, Schema, TYPE_BOOL, TYPE_BYTE, TYPE_FLOAT, TYPE_INT, TYPE_INT64,
    TYPE_STRING, TYPE_UINT, TYPE_UINT64,
};

pub(crate) const CANVAS_BYTES: u64 = 512 * 1024 * 1024;
pub(crate) const SCHEMA_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const MESSAGE_BYTES: u64 = 256 * 1024 * 1024;
pub(crate) const CHUNKS: usize = 1024;
const SCHEMA_ITEMS: usize = 262_144;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub depth: usize,
    pub items: usize,
}

// Initial safety ceilings, not a calibration against representative large designs.
pub(crate) const VALUES: Limits = Limits {
    depth: 64,
    items: 16_777_216,
};

pub(crate) fn exceeded(kind: &str, limit: impl std::fmt::Display) -> FigError {
    FigError::new(
        crate::ErrorKind::ParseBudget,
        format!(
            "parse budget exceeded: {kind} (limit {limit}); use a smaller local copy or report this limit for review"
        ),
    )
}

struct Budget {
    remaining: usize,
    limit: usize,
    kind: &'static str,
}

impl Budget {
    fn new(limit: usize, kind: &'static str) -> Self {
        Self {
            remaining: limit,
            limit,
            kind,
        }
    }

    fn check(&self, count: usize) -> Result<(), FigError> {
        if count > self.remaining {
            return Err(exceeded(self.kind, self.limit));
        }
        Ok(())
    }

    fn spend(&mut self, count: usize) -> Result<(), FigError> {
        self.check(count)?;
        self.remaining -= count;
        Ok(())
    }
}

fn invalid_schema(_: ()) -> FigError {
    FigError::new(crate::ErrorKind::InvalidData, "invalid kiwi schema")
}

fn invalid_message(_: ()) -> FigError {
    FigError::new(
        crate::ErrorKind::InvalidData,
        "message does not match its schema",
    )
}

pub(crate) fn schema(bytes: &[u8]) -> Result<(), FigError> {
    schema_with_limit(bytes, SCHEMA_ITEMS)
}

fn schema_with_limit(bytes: &[u8], limit: usize) -> Result<(), FigError> {
    let mut bb = ByteBuffer::new(bytes);
    let mut budget = Budget::new(limit, "schema items");
    let definitions = bb.read_var_uint().map_err(invalid_schema)? as usize;
    budget.spend(definitions)?;
    for _ in 0..definitions {
        bb.read_string().map_err(invalid_schema)?;
        bb.read_byte().map_err(invalid_schema)?;
        let fields = bb.read_var_uint().map_err(invalid_schema)? as usize;
        budget.spend(fields)?;
        for _ in 0..fields {
            bb.read_string().map_err(invalid_schema)?;
            bb.read_var_int().map_err(invalid_schema)?;
            bb.read_bool().map_err(invalid_schema)?;
            bb.read_var_uint().map_err(invalid_schema)?;
        }
    }
    Ok(())
}

pub(crate) fn message(
    schema: &Schema,
    root: i32,
    bytes: &[u8],
    limits: Limits,
) -> Result<(), FigError> {
    let mut budget = Budget::new(limits.items, "decoded items");
    value(
        schema,
        root,
        &mut ByteBuffer::new(bytes),
        1,
        limits,
        &mut budget,
    )
}

fn enter(depth: usize, limits: Limits, budget: &mut Budget) -> Result<(), FigError> {
    if depth > limits.depth {
        return Err(exceeded("value depth", limits.depth));
    }
    budget.spend(1)
}

fn child_depth(depth: usize, limits: Limits) -> Result<usize, FigError> {
    if depth >= limits.depth {
        return Err(exceeded("value depth", limits.depth));
    }
    depth
        .checked_add(1)
        .ok_or_else(|| exceeded("value depth", limits.depth))
}

fn value(
    schema: &Schema,
    type_id: i32,
    bb: &mut ByteBuffer<'_>,
    depth: usize,
    limits: Limits,
    budget: &mut Budget,
) -> Result<(), FigError> {
    enter(depth, limits, budget)?;
    match type_id {
        TYPE_BOOL | TYPE_BYTE | TYPE_INT | TYPE_UINT | TYPE_FLOAT | TYPE_STRING | TYPE_INT64
        | TYPE_UINT64 => {
            // Only primitives: upstream composite skipping has no recursion budget.
            schema.skip(bb, type_id).map_err(invalid_message)?;
        }
        _ => {
            let def = schema
                .defs
                .get(type_id as usize)
                .ok_or_else(|| invalid_message(()))?;
            match def.kind {
                DefKind::Enum => {
                    let id = bb.read_var_uint().map_err(invalid_message)?;
                    let index = def
                        .field_value_to_index
                        .get(&id)
                        .ok_or_else(|| invalid_message(()))?;
                    def.fields.get(*index).ok_or_else(|| invalid_message(()))?;
                }
                DefKind::Struct => {
                    budget.check(def.fields.len())?;
                    for field in &def.fields {
                        field_value(
                            schema,
                            field,
                            bb,
                            child_depth(depth, limits)?,
                            limits,
                            budget,
                        )?;
                    }
                }
                DefKind::Message => loop {
                    let id = bb.read_var_uint().map_err(invalid_message)?;
                    if id == 0 {
                        break;
                    }
                    let index = def
                        .field_value_to_index
                        .get(&id)
                        .ok_or_else(|| invalid_message(()))?;
                    let field = def.fields.get(*index).ok_or_else(|| invalid_message(()))?;
                    field_value(
                        schema,
                        field,
                        bb,
                        child_depth(depth, limits)?,
                        limits,
                        budget,
                    )?;
                },
            }
        }
    }
    Ok(())
}

fn field_value(
    schema: &Schema,
    field: &Field,
    bb: &mut ByteBuffer<'_>,
    depth: usize,
    limits: Limits,
    budget: &mut Budget,
) -> Result<(), FigError> {
    if field.is_array {
        enter(depth, limits, budget)?;
        let len = bb.read_var_uint().map_err(invalid_message)? as usize;
        budget.check(len)?;
        for _ in 0..len {
            value(
                schema,
                field.type_id,
                bb,
                child_depth(depth, limits)?,
                limits,
                budget,
            )?;
        }
        Ok(())
    } else {
        value(schema, field.type_id, bb, depth, limits, budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwi_schema::{Def, Value};

    fn field(name: &str, type_id: i32, is_array: bool) -> Field {
        Field {
            name: name.into(),
            type_id,
            is_array,
            value: 1,
        }
    }

    #[test]
    fn finite_values_obey_depth_boundaries_and_reset_after_failure() {
        // Two distinct, acyclic definitions and one primitive; no cyclic-schema payload.
        let schema = Schema::new(vec![
            Def::new(
                "Message".into(),
                DefKind::Struct,
                vec![field("child", 1, false)],
            ),
            Def::new(
                "Child".into(),
                DefKind::Struct,
                vec![field("number", TYPE_UINT, false)],
            ),
        ]);
        for depth in [4, 3] {
            message(&schema, 0, &[7], Limits { depth, items: 3 }).unwrap();
        }
        let error = message(&schema, 0, &[7], Limits { depth: 2, items: 3 }).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("parse budget exceeded: value depth (limit 2)")
        );
        message(&schema, 0, &[7], Limits { depth: 3, items: 3 }).unwrap();
        assert!(Value::decode(&schema, 0, &[7]).is_ok());
    }

    #[test]
    fn array_work_is_checked_before_elements_and_has_exact_boundaries() {
        let schema = Schema::new(vec![Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("values", TYPE_UINT, true)],
        )]);
        let bytes = [1, 3, 4, 5, 6, 0]; // root + array + three integers = five items
        for items in [6, 5] {
            message(&schema, 0, &bytes, Limits { depth: 3, items }).unwrap();
        }
        let limits = Limits { depth: 3, items: 4 };
        let error = message(&schema, 0, &bytes, limits).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("parse budget exceeded: decoded items (limit 4)")
        );
        // A small declared array over budget is rejected before reading its absent elements.
        assert_eq!(
            message(&schema, 0, &[1, 3], limits)
                .unwrap_err()
                .to_string(),
            error.to_string()
        );
        message(&schema, 0, &bytes, Limits { depth: 3, items: 5 }).unwrap();
        assert!(message(&schema, 0, &[1, 1, 4, 0], limits).is_ok());
    }

    #[test]
    fn schema_counts_definitions_and_fields_before_decoding_them() {
        let schema = Schema::new(vec![Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("value", TYPE_UINT, false)],
        )]);
        let bytes = schema.encode(); // one definition + one field
        for limit in [3, 2] {
            schema_with_limit(&bytes, limit).unwrap();
        }
        assert!(
            schema_with_limit(&bytes, 1)
                .unwrap_err()
                .to_string()
                .contains("schema items (limit 1)")
        );
        schema_with_limit(&bytes, 2).unwrap();
    }

    #[test]
    fn counters_do_not_overflow_and_empty_arrays_need_no_element_depth() {
        let mut budget = Budget::new(2, "test items");
        assert!(budget.spend(usize::MAX).is_err());
        budget.spend(2).unwrap();
        assert!(budget.spend(1).is_err());
        let schema = Schema::new(vec![Def::new(
            "Message".into(),
            DefKind::Message,
            vec![field("values", TYPE_UINT, true)],
        )]);
        message(&schema, 0, &[1, 0, 0], Limits { depth: 2, items: 2 }).unwrap();
    }
}
