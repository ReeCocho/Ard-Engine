use ard_graphics_api::prelude::Model;
use serde::{Deserialize, Serialize};

use crate::components::{
    renderable::RenderableData,
    transform::{Children, Parent, Transform},
};

use crate::game_object_def;

game_object_def!(
    StaticObject,
    Transform
    Parent
    Children
    Model
    RenderableData
);
