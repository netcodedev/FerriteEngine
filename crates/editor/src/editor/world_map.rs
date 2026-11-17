//! The terrain's world map in the inspector: the whole world from above,
//! as the terrain's "map" property picks (height or a climate map). It
//! follows the terrain: a changed seed or world setting brings a new map.

use ferrite::{
    core::{
        entity::EntityHandle,
        primitives::{Offset, Position, Size},
        scene::Scene,
    },
    terrain::Terrain,
};
use ferrite_ui::ui::{element_handle::UIElementHandle, elements::image::Image, UIElement};
use glfw::{Glfw, WindowEvent};

pub struct WorldMapView {
    handle: UIElementHandle,
    offset: Offset,
    size: Size,
    z: f32,
    terrain: EntityHandle,
    image: Option<Image>,
    /// Version of the map shown.
    shown: Option<u64>,
}

impl WorldMapView {
    pub fn new(terrain: EntityHandle, width: f32) -> Self {
        Self {
            handle: UIElementHandle::new(),
            offset: Offset::default(),
            size: Size::new(width, width),
            z: 0.0,
            terrain,
            image: None,
            shown: None,
        }
    }
}

impl UIElement for WorldMapView {
    fn update(&mut self, scene: &mut Scene) {
        let map = scene
            .get_entity_mut(&self.terrain)
            .and_then(|e| e.get_component_mut::<Terrain>())
            .and_then(|terrain| terrain.world_map());
        if let Some((version, picture)) = map {
            if self.shown != Some(version) {
                let side = self.size.width;
                match &mut self.image {
                    Some(image) => image.set_rgba(&picture),
                    None => {
                        let mut image = Image::from_rgba(Position::default(), Size::new(side, side), &picture);
                        image.set_z_index(self.z + 1.0);
                        self.image = Some(image);
                    }
                }
                self.shown = Some(version);
            }
        }
        if let Some(image) = &mut self.image {
            image.set_offset(self.offset.clone());
        }
    }

    fn render(&self) {
        if let Some(image) = &self.image {
            image.render();
        }
    }

    fn handle_events(&mut self, _: &mut Scene, _: &mut glfw::Window, _: &mut Glfw, _: &WindowEvent) -> bool {
        false
    }

    fn add_child(&mut self, _child: Box<dyn UIElement>) {}
    fn add_child_to(&mut self, _p: UIElementHandle, _c: Box<dyn UIElement>) {}
    fn contains_child(&self, _h: &UIElementHandle) -> bool {
        false
    }
    fn get_child(&self, _h: &UIElementHandle) -> Option<&Box<dyn UIElement>> {
        None
    }
    fn get_child_mut(&mut self, _h: &UIElementHandle) -> Option<&mut Box<dyn UIElement>> {
        None
    }
    fn get_handle(&self) -> &UIElementHandle {
        &self.handle
    }
    fn get_offset(&self) -> &Offset {
        &self.offset
    }
    fn set_offset(&mut self, offset: Offset) {
        self.offset = offset;
    }
    fn get_size(&self) -> &Size {
        &self.size
    }
    fn set_z_index(&mut self, z: f32) {
        self.z = z;
        if let Some(image) = &mut self.image {
            image.set_z_index(z + 1.0);
        }
    }
}
