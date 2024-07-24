use std::num::NonZeroUsize;

use ard_engine::{
    ecs::prelude::*,
    math::{Mat4, Vec4},
    physics::{
        collider::{self, Collider, ColliderHandle},
        engine::PhysicsEngine,
    },
    render::{DebugDraw, DebugDrawing, EntitySelected, PreRender},
    transform::Model,
};

#[derive(Resource, Default)]
pub enum Selected {
    #[default]
    None,
    Entity(Entity),
}

#[derive(SystemState)]
pub struct SelectEntitySystem;

const COLLIDER_GIZMO_COLOR: Vec4 = Vec4::new(0.0, 1.0, 0.0, 1.0);

impl SelectEntitySystem {
    fn selected_entity(
        &mut self,
        evt: EntitySelected,
        _: Commands,
        _: Queries<()>,
        res: Res<(Write<Selected>,)>,
    ) {
        let mut selected = res.get_mut::<Selected>().unwrap();
        *selected = Selected::Entity(evt.0);
    }

    fn pre_render(
        &mut self,
        _: PreRender,
        _: Commands,
        queries: Queries<(Read<Model>, Read<ColliderHandle>, Read<Collider>)>,
        res: Res<(Read<Selected>, Write<DebugDrawing>, Read<PhysicsEngine>)>,
    ) {
        let mut debug = res.get_mut::<DebugDrawing>().unwrap();
        let phys_engine = res.get::<PhysicsEngine>().unwrap();
        let selected = res.get::<Selected>().unwrap();

        let selected = match *selected {
            Selected::Entity(entity) => entity,
            _ => return,
        };

        let model = Model(
            queries
                .get::<Read<Model>>(selected)
                .map(|mdl| mdl.0)
                .unwrap_or(Mat4::IDENTITY),
        );

        if let Some(query) = queries.get::<(Option<Read<ColliderHandle>>, Read<Collider>)>(selected)
        {
            let (handle, collider) = *query;
            let (pos, rot) = match handle {
                Some(handle) => phys_engine.colliders(|set| {
                    set.get(handle.handle())
                        .map(|col| (col.translation().xyz().into(), (*col.rotation()).into()))
                        .unwrap_or((model.position(), model.rotation()))
                }),
                None => {
                    let col_model = Model(model.0 * Mat4::from_translation(collider.offset));
                    (col_model.position(), col_model.rotation())
                }
            };

            let model = Mat4::from_rotation_translation(rot, pos.into());

            match &collider.shape {
                collider::Shape::Box { half_extents } => debug.draw(DebugDraw {
                    color: COLLIDER_GIZMO_COLOR,
                    shape: ard_engine::render::shape::Shape::Box {
                        min_pt: -*half_extents,
                        max_pt: *half_extents,
                        model,
                    },
                }),
                collider::Shape::Ball { radius } => debug.draw(DebugDraw {
                    color: COLLIDER_GIZMO_COLOR,
                    shape: ard_engine::render::shape::Shape::Sphere {
                        radius: *radius,
                        model,
                        segments: NonZeroUsize::new(32).unwrap(),
                    },
                }),
                collider::Shape::Cylinder { height, radius } => debug.draw(DebugDraw {
                    color: COLLIDER_GIZMO_COLOR,
                    shape: ard_engine::render::shape::Shape::Cylinder {
                        radius: *radius,
                        height: *height,
                        model,
                        segments: NonZeroUsize::new(32).unwrap(),
                    },
                }),
                collider::Shape::Cone { height, radius } => debug.draw(DebugDraw {
                    color: COLLIDER_GIZMO_COLOR,
                    shape: ard_engine::render::shape::Shape::Cone {
                        radius: *radius,
                        height: *height,
                        model,
                        segments: NonZeroUsize::new(32).unwrap(),
                    },
                }),
                collider::Shape::Capsule { height, radius } => debug.draw(DebugDraw {
                    color: COLLIDER_GIZMO_COLOR,
                    shape: ard_engine::render::shape::Shape::Capsule {
                        radius: *radius,
                        height: *height,
                        model,
                        segments: NonZeroUsize::new(32).unwrap(),
                    },
                }),
            }
        }
    }
}

impl From<SelectEntitySystem> for System {
    fn from(value: SelectEntitySystem) -> Self {
        SystemBuilder::new(value)
            .with_handler(SelectEntitySystem::selected_entity)
            .with_handler(SelectEntitySystem::pre_render)
            .build()
    }
}
