use crate::core::{
    entity::EntityHandle,
    renderer::{plane::Plane, text::Text},
    scene::Scene,
};

use super::primitives::{Offset, Position, Size};

pub mod input;

type GetFn = dyn Fn(&Option<EntityHandle>, &mut Scene) -> String;
type SetFn = dyn FnMut(&Option<EntityHandle>, &mut Scene, String);

pub struct Input {
    position: Position,
    size: Size,
    offset: Offset,
    pub is_hovering: bool,
    pub is_focused: bool,
    pub content: String,
    text: Text,
    get_fn: Option<Box<GetFn>>,
    set_fn: Option<Box<SetFn>>,
    plane: Plane,
    stencil_plane: Plane,
    entity_handle: Option<EntityHandle>,
}

pub struct InputBuilder {
    position: Position,
    size: Size,
    content: String,
    get_fn: Option<Box<GetFn>>,
    set_fn: Option<Box<SetFn>>,
    entity_handle: Option<EntityHandle>,
}
