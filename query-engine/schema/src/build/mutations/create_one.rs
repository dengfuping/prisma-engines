use super::*;
use crate::{
    Identifier, IdentifierType, InputField, InputObjectType, InputType, OutputField, OutputType, QueryInfo, QueryTag,
};
use constants::*;
use input_types::fields::{arguments, data_input_mapper::*};
use output_types::objects;
use query_structure::{Model, RelationFieldRef};

/// Builds a create mutation field (e.g. createUser) for given model.
pub(crate) fn create_one(ctx: &QuerySchema, model: Model) -> OutputField<'_> {
    let field_name = format!("createOne{}", model.name());
    let cloned_model = model.clone();
    let model_id = model.id;

    field(
        field_name,
        move || create_one_arguments(ctx, model),
        OutputType::object(objects::model::model_object_type(ctx, cloned_model)),
        Some(QueryInfo {
            model: Some(model_id),
            tag: QueryTag::CreateOne,
        }),
    )
}

/// Builds "data" argument intended for the create field.
/// The data argument is not present if no data can be created.
pub(crate) fn create_one_arguments(ctx: &QuerySchema, model: Model) -> Vec<InputField<'_>> {
    let any_field_required = model
        .fields()
        .all()
        .any(|f| f.is_required() && f.as_scalar().map(|f| f.default_value().is_none()).unwrap_or(true));

    let create_types = create_one_input_types(ctx, model, None);
    let data_field = input_field(args::DATA, create_types, None).optional_if(!any_field_required);

    std::iter::once(data_field)
        .chain(arguments::relation_load_strategy_argument(ctx))
        .collect()
}

pub(crate) fn create_one_input_types(
    ctx: &QuerySchema,
    model: Model,
    parent_field: Option<RelationFieldRef>,
) -> Vec<InputType<'_>> {
    let checked_input = InputType::object(checked_create_input_type(ctx, model.clone(), parent_field.clone()));
    let unchecked_input = InputType::object(unchecked_create_input_type(ctx, model, parent_field));
    vec![checked_input, unchecked_input]
}

/// Builds the create input type (<x>CreateInput / <x>CreateWithout<y>Input)
/// Also valid for nested inputs. A nested input is constructed if the `parent_field` is provided.
/// "Checked" input refers to disallowing writing relation scalars directly, as it can lead to unintended
/// data integrity violations if used incorrectly.
fn checked_create_input_type(
    ctx: &QuerySchema,
    model: Model,
    parent_field: Option<RelationFieldRef>,
) -> InputObjectType<'_> {
    // We allow creation from both sides of the relation - which would lead to an endless loop of input types
    // if we would allow to create the parent from a child create that is already a nested create.
    // To solve it, we remove the parent relation from the input ("Without<Parent>").
    let ident = Identifier::new_prisma(IdentifierType::CheckedCreateInput(
        model.clone(),
        parent_field.as_ref().map(|pf| pf.related_field()),
    ));

    let mut input_object = init_input_object_type(ident);
    input_object.set_container(model.clone());
    input_object.set_fields(move || {
        let mut filtered_fields = filter_checked_create_fields(&model, parent_field.clone());
        let field_mapper = CreateDataInputFieldMapper::new_checked();
        field_mapper.map_all(ctx, &mut filtered_fields)
    });
    input_object
}

/// Builds the create input type (<x>UncheckedCreateInput / <x>UncheckedCreateWithout<y>Input)
/// Also valid for nested inputs. A nested input is constructed if the `parent_field` is provided.
/// "Unchecked" input refers to allowing to write _all_ scalars on a model directly, which can
/// lead to unintended data integrity violations if used incorrectly.
fn unchecked_create_input_type(
    ctx: &QuerySchema,
    model: Model,
    parent_field: Option<RelationFieldRef>,
) -> InputObjectType<'_> {
    // We allow creation from both sides of the relation - which would lead to an endless loop of input types
    // if we would allow to create the parent from a child create that is already a nested create.
    // To solve it, we remove the parent relation from the input ("Without<Parent>").
    let ident = Identifier::new_prisma(IdentifierType::UncheckedCreateInput(
        model.clone(),
        parent_field.as_ref().map(|pf| pf.related_field()),
    ));

    let mut input_object = init_input_object_type(ident);
    input_object.set_container(model.clone());
    input_object.set_fields(move || {
        let mut filtered_fields = filter_unchecked_create_fields(&model, parent_field.as_ref());
        let field_mapper = CreateDataInputFieldMapper::new_unchecked();
        field_mapper.map_all(ctx, &mut filtered_fields)
    });
    input_object
}

/// Filters the given model's fields down to the allowed ones for checked create.
fn filter_checked_create_fields(
    model: &Model,
    parent_field: Option<RelationFieldRef>,
) -> impl Iterator<Item = ModelField> + '_ {
    model.fields().filter_all(move |field| {
        match field {
            // Scalars must be writable and not an autogenerated ID, which are disallowed for checked inputs
            // regardless of whether or not the connector supports it.
            ModelField::Scalar(sf) => !sf.is_auto_generated_int_id() && !sf.is_read_only(),

            // If the relation field `rf` is the one that was traversed to by the parent relation field `parent_field`,
            // then exclude it for checked inputs - this prevents endless nested type circles that are useless to offer as API.
            ModelField::Relation(rf) => {
                let field_was_traversed_to = parent_field
                    .as_ref()
                    .filter(|pf| pf.related_field().id == rf.id)
                    .is_some();
                !field_was_traversed_to
            }

            // Always keep composites
            ModelField::Composite(_) => true,
        }
    })
}

/// Filters the given model's fields down to the allowed ones for unchecked create.
fn filter_unchecked_create_fields<'a>(
    model: &'a Model,
    parent_field: Option<&'a RelationFieldRef>,
) -> impl Iterator<Item = ModelField> + 'a {
    let linking_fields = if let Some(parent_field) = parent_field {
        let child_field = parent_field.related_field();
        if child_field.is_inlined_on_enclosing_model() {
            child_field
                .linking_fields()
                .as_scalar_fields()
                .expect("Expected linking fields to be scalar.")
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    model.fields().filter_all(move |field| match field {
        // In principle, all scalars are writable for unchecked inputs. However, it still doesn't make any sense to be able to write the scalars that
        // link the model to the parent record in case of a nested unchecked create, as this would introduce complexities we don't want to deal with right now.
        ModelField::Scalar(sf) => !linking_fields.contains(sf),

        // If the relation field `rf` is the one that was traversed to by the parent relation field `parent_field`,
        // then exclude it for checked inputs - this prevents endless nested type circles that are useless to offer as API.
        //
        // Additionally, only relations that point to other models and are NOT inlined on the currently in scope model are allowed in the unchecked input, because if they are
        // inlined, they are written only as scalars for unchecked, not via the relation API (`connect`, nested `create`, etc.).
        ModelField::Relation(rf) => {
            let is_not_inlined = !rf.is_inlined_on_enclosing_model();
            let field_was_not_traversed_to = parent_field
                .filter(|pf| pf.related_field().name() == rf.name())
                .is_none();

            field_was_not_traversed_to && is_not_inlined
        }

        // Always keep composites
        ModelField::Composite(_) => true,
    })
}
