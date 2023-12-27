use api::{
    buffer::Buffer,
    command_buffer::{
        BlitDestination, BlitSource, BufferCubeMapCopy, BufferTextureCopy, Command,
        CopyBufferToBuffer,
    },
    cube_map::CubeMap,
    descriptor_set::DescriptorSet,
    render_pass::{ColorAttachmentSource, RenderPassDescriptor},
    texture::{Blit, Texture},
    types::{QueueTypes, SharingMode},
};

use super::{
    semaphores::{SemaphoreTracker, WaitInfo},
    usage::{PipelineTracker, SubResource, SubResourceUsage, UsageScope},
};
use crate::descriptor_set::BoundValue;
use ash::vk;

pub(crate) struct TrackState<'a, 'b> {
    pub device: &'a ash::Device,
    pub command_buffer: vk::CommandBuffer,
    /// Index of the command to detect the resources of.
    pub index: usize,
    /// Command list with all commands of a submit.
    pub commands: &'a [Command<'a, crate::VulkanBackend>],
    /// Used to detect inter-command dependencies.
    pub pipeline_tracker: &'a mut PipelineTracker<'b>,
    /// Used by `resc_state` to track inter-queue dependencies.
    pub semaphores: &'a mut SemaphoreTracker,
}

/// Given the index of a command in a command list, tracks resources based off the type of
/// detected command.
pub(crate) unsafe fn track_resources(mut state: TrackState) {
    match &state.commands[state.index] {
        Command::BeginRenderPass(descriptor) => track_render_pass(&mut state, descriptor),
        Command::Dispatch(_, _, _) => track_dispatch(&mut state),
        Command::CopyBufferToBuffer(copy_info) => {
            track_buffer_to_buffer_copy(&mut state, copy_info)
        }
        Command::CopyBufferToTexture {
            buffer,
            texture,
            copy,
        } => track_buffer_to_texture_copy(&mut state, buffer, texture, copy),
        Command::CopyTextureToBuffer {
            buffer,
            texture,
            copy,
        } => track_texture_to_buffer_copy(&mut state, buffer, texture, copy),
        Command::CopyBufferToCubeMap {
            buffer,
            cube_map,
            copy,
        } => track_buffer_to_cube_map_copy(&mut state, buffer, cube_map, copy),
        Command::CopyCubeMapToBuffer {
            cube_map,
            buffer,
            copy,
        } => track_cube_map_to_buffer_copy(&mut state, cube_map, buffer, copy),
        Command::Blit { src, dst, blit, .. } => track_blit(&mut state, src, dst, blit),
        // All other commands do not need state tracking
        _ => {}
    }
}

unsafe fn track_render_pass(
    state: &mut TrackState,
    descriptor: &RenderPassDescriptor<'_, crate::VulkanBackend>,
) {
    let mut scope = UsageScope::default();

    // Track color attachments used in the pass
    for attachment in &descriptor.color_attachments {
        let (subresource, layout) = match attachment.source {
            ColorAttachmentSource::SurfaceImage(image) => {
                // Surface image has special semaphores
                let semaphores = image.internal().semaphores();
                state
                    .semaphores
                    .register_signal(semaphores.presentable, None);
                state.semaphores.register_wait(
                    semaphores.available,
                    WaitInfo {
                        value: None,
                        stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                    },
                );

                (
                    SubResource::Texture {
                        texture: image.internal().image(),
                        queue_types: QueueTypes::all(),
                        sharing: SharingMode::Concurrent,
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        array_elem: 0,
                        mip_level: 0,
                    },
                    vk::ImageLayout::PRESENT_SRC_KHR,
                )
            }
            ColorAttachmentSource::Texture {
                texture,
                array_element,
                mip_level,
            } => (
                SubResource::Texture {
                    texture: texture.internal().image,
                    queue_types: texture.queue_types(),
                    sharing: texture.sharing_mode(),
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    array_elem: array_element as u32,
                    mip_level: mip_level as u32,
                },
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
            ColorAttachmentSource::CubeMap {
                cube_map,
                array_element,
                mip_level,
                ..
            } => (
                SubResource::CubeMap {
                    cube_map: cube_map.internal().image,
                    queue_types: cube_map.queue_types(),
                    sharing: cube_map.sharing_mode(),
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    array_elem: array_element as u32,
                    mip_level: mip_level as u32,
                },
                vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            ),
        };
        scope.use_resource(
            subresource,
            SubResourceUsage {
                access: vk::AccessFlags::COLOR_ATTACHMENT_WRITE
                    | vk::AccessFlags::COLOR_ATTACHMENT_READ,
                stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                layout,
            },
        );
    }

    // Track depth stencil attachment
    if let Some(attachment) = &descriptor.depth_stencil_attachment {
        let internal = attachment.texture.internal();
        scope.use_resource(
            SubResource::Texture {
                texture: internal.image,
                queue_types: attachment.texture.queue_types(),
                sharing: attachment.texture.sharing_mode(),
                aspect_mask: internal.aspect_flags,
                array_elem: attachment.array_element as u32,
                mip_level: attachment.mip_level as u32,
            },
            SubResourceUsage {
                access: vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_READ
                    | vk::AccessFlags::DEPTH_STENCIL_ATTACHMENT_WRITE,
                stage: vk::PipelineStageFlags::EARLY_FRAGMENT_TESTS
                    | vk::PipelineStageFlags::LATE_FRAGMENT_TESTS,
                layout: vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
            },
        );
    }

    // Track everything else
    for command in &state.commands[state.index..] {
        match command {
            Command::BindVertexBuffers { binds, .. } => {
                for bind in binds {
                    scope.use_resource(
                        SubResource::Buffer {
                            buffer: bind.buffer.internal().buffer,
                            sharing: bind.buffer.sharing_mode(),
                            queue_types: bind.buffer.queue_types(),
                            aligned_size: bind.buffer.internal().aligned_size as usize,
                            array_elem: bind.array_element as u32,
                        },
                        SubResourceUsage {
                            access: vk::AccessFlags::VERTEX_ATTRIBUTE_READ,
                            stage: vk::PipelineStageFlags::VERTEX_INPUT,
                            layout: vk::ImageLayout::UNDEFINED,
                        },
                    );
                }
            }
            Command::BindIndexBuffer {
                buffer,
                array_element,
                ..
            } => {
                scope.use_resource(
                    SubResource::Buffer {
                        buffer: buffer.internal().buffer,
                        queue_types: buffer.queue_types(),
                        sharing: buffer.sharing_mode(),
                        aligned_size: buffer.internal().aligned_size as usize,
                        array_elem: *array_element as u32,
                    },
                    SubResourceUsage {
                        access: vk::AccessFlags::INDEX_READ,
                        stage: vk::PipelineStageFlags::VERTEX_INPUT,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                );
            }
            Command::BindDescriptorSets { sets, .. } => {
                track_descriptor_sets(
                    sets,
                    vk::PipelineStageFlags::VERTEX_SHADER | vk::PipelineStageFlags::FRAGMENT_SHADER,
                    &mut scope,
                );
            }
            Command::DrawIndexedIndirect {
                buffer,
                array_element,
                ..
            } => {
                scope.use_resource(
                    SubResource::Buffer {
                        buffer: buffer.internal().buffer,
                        queue_types: buffer.queue_types(),
                        sharing: buffer.sharing_mode(),
                        aligned_size: buffer.internal().aligned_size as usize,
                        array_elem: *array_element as u32,
                    },
                    SubResourceUsage {
                        access: vk::AccessFlags::INDIRECT_COMMAND_READ,
                        stage: vk::PipelineStageFlags::DRAW_INDIRECT,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                );
            }
            Command::DrawIndexedIndirectCount {
                draw_buffer,
                draw_array_element,
                count_buffer,
                count_array_element,
                ..
            } => {
                scope.use_resource(
                    SubResource::Buffer {
                        buffer: draw_buffer.internal().buffer,
                        queue_types: draw_buffer.queue_types(),
                        sharing: draw_buffer.sharing_mode(),
                        aligned_size: draw_buffer.internal().aligned_size as usize,
                        array_elem: *draw_array_element as u32,
                    },
                    SubResourceUsage {
                        access: vk::AccessFlags::INDIRECT_COMMAND_READ,
                        stage: vk::PipelineStageFlags::DRAW_INDIRECT,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                );

                scope.use_resource(
                    SubResource::Buffer {
                        buffer: count_buffer.internal().buffer,
                        queue_types: count_buffer.queue_types(),
                        sharing: count_buffer.sharing_mode(),
                        aligned_size: count_buffer.internal().aligned_size as usize,
                        array_elem: *count_array_element as u32,
                    },
                    SubResourceUsage {
                        access: vk::AccessFlags::INDIRECT_COMMAND_READ,
                        stage: vk::PipelineStageFlags::DRAW_INDIRECT,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                );
            }
            Command::EndRenderPass => break,
            _ => {}
        }
    }

    // Submit usage scope
    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_dispatch(state: &mut TrackState) {
    // Find the index of the bound pipeline
    let idx = {
        let mut idx = None;
        for (i, command) in state.commands[..=state.index].iter().enumerate().rev() {
            match command {
                Command::BindComputePipeline(_) => {
                    idx = Some(i);
                    break;
                }
                Command::BeginComputePass => break,
                _ => {}
            }
        }

        match idx {
            Some(idx) => idx,
            // No bound pipeline so no state track needed
            None => return,
        }
    };

    // Determine how many sets are used by the active pipeline
    let mut total_bound = 0;
    let mut bound = {
        let pipeline = match &state.commands[idx] {
            Command::BindComputePipeline(pipeline) => pipeline,
            // Unreachable because of early return in previous pass
            _ => unreachable!(),
        };
        let mut bound = Vec::with_capacity(pipeline.layouts().len());
        bound.resize(pipeline.layouts().len(), false);
        bound
    };

    let mut scope = UsageScope::default();

    // Determine which sets are actually used
    for command in state.commands[idx..=state.index].iter().rev() {
        // Break early if every set is bound
        if total_bound == bound.len() {
            break;
        }

        // Grab bind info. Skip other commands
        let (sets, first) = match command {
            Command::BindDescriptorSets { sets, first, .. } => (sets, *first),
            _ => continue,
        };

        // Track sets
        for (i, set_slot) in (first..(first + sets.len())).enumerate() {
            // Skip if the set slot is already bound
            if bound[set_slot] {
                continue;
            }

            // Track
            track_descriptor_set(sets[i], vk::PipelineStageFlags::COMPUTE_SHADER, &mut scope);
            bound[set_slot] = true;
            total_bound += 1;
        }
    }

    // Submit pipeline values
    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_buffer_to_buffer_copy(
    state: &mut TrackState,
    copy: &CopyBufferToBuffer<'_, crate::VulkanBackend>,
) {
    // Barrier check
    let src = copy.src.internal();
    let dst = copy.dst.internal();
    let mut scope = UsageScope::default();

    scope.use_resource(
        SubResource::Buffer {
            buffer: src.buffer,
            queue_types: copy.src.queue_types(),
            sharing: copy.src.sharing_mode(),
            array_elem: copy.src_array_element as u32,
            aligned_size: copy.src.internal().aligned_size as usize,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::Buffer {
            buffer: dst.buffer,
            queue_types: copy.dst.queue_types(),
            sharing: copy.dst.sharing_mode(),
            array_elem: copy.dst_array_element as u32,
            aligned_size: copy.dst.internal().aligned_size as usize,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_buffer_to_texture_copy(
    state: &mut TrackState,
    buffer: &Buffer<crate::VulkanBackend>,
    texture: &Texture<crate::VulkanBackend>,
    copy: &BufferTextureCopy,
) {
    // Barrier check
    let buffer_int = buffer.internal();
    let texture_int = texture.internal();
    let mut scope = UsageScope::default();

    scope.use_resource(
        SubResource::Buffer {
            buffer: buffer_int.buffer,
            queue_types: buffer.queue_types(),
            sharing: buffer.sharing_mode(),
            array_elem: copy.buffer_array_element as u32,
            aligned_size: buffer.internal().aligned_size as usize,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::Texture {
            texture: texture_int.image,
            queue_types: texture.queue_types(),
            sharing: texture.sharing_mode(),
            aspect_mask: texture_int.aspect_flags,
            array_elem: copy.texture_array_element as u32,
            mip_level: copy.texture_mip_level as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_texture_to_buffer_copy(
    state: &mut TrackState,
    buffer: &Buffer<crate::VulkanBackend>,
    texture: &Texture<crate::VulkanBackend>,
    copy: &BufferTextureCopy,
) {
    // Barrier check
    let buffer_int = buffer.internal();
    let texture_int = texture.internal();
    let mut scope = UsageScope::default();

    scope.use_resource(
        SubResource::Buffer {
            buffer: buffer_int.buffer,
            queue_types: buffer.queue_types(),
            sharing: buffer.sharing_mode(),
            array_elem: copy.buffer_array_element as u32,
            aligned_size: buffer.internal().aligned_size as usize,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::Texture {
            texture: texture_int.image,
            queue_types: texture.queue_types(),
            sharing: texture.sharing_mode(),
            aspect_mask: texture_int.aspect_flags,
            array_elem: copy.texture_array_element as u32,
            mip_level: copy.texture_mip_level as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_buffer_to_cube_map_copy(
    state: &mut TrackState,
    buffer: &Buffer<crate::VulkanBackend>,
    cube_map: &CubeMap<crate::VulkanBackend>,
    copy: &BufferCubeMapCopy,
) {
    // Barrier check
    let buffer_int = buffer.internal();
    let cube_map_int = cube_map.internal();
    let mut scope = UsageScope::default();

    scope.use_resource(
        SubResource::Buffer {
            buffer: buffer_int.buffer,
            queue_types: buffer.queue_types(),
            sharing: buffer.sharing_mode(),
            array_elem: copy.buffer_array_element as u32,
            aligned_size: buffer_int.aligned_size as usize,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::CubeMap {
            cube_map: cube_map_int.image,
            queue_types: cube_map.queue_types(),
            sharing: cube_map.sharing_mode(),
            aspect_mask: cube_map_int.aspect_flags,
            array_elem: copy.cube_map_array_element as u32,
            mip_level: copy.cube_map_mip_level as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_cube_map_to_buffer_copy(
    state: &mut TrackState,
    cube_map: &CubeMap<crate::VulkanBackend>,
    buffer: &Buffer<crate::VulkanBackend>,
    copy: &BufferCubeMapCopy,
) {
    // Barrier check
    let buffer_int = buffer.internal();
    let cube_map_int = cube_map.internal();
    let mut scope = UsageScope::default();

    scope.use_resource(
        SubResource::Buffer {
            buffer: buffer_int.buffer,
            queue_types: buffer.queue_types(),
            sharing: buffer.sharing_mode(),
            array_elem: copy.buffer_array_element as u32,
            aligned_size: buffer_int.aligned_size as usize,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );
    scope.use_resource(
        SubResource::CubeMap {
            cube_map: cube_map_int.image,
            queue_types: cube_map.queue_types(),
            sharing: cube_map.sharing_mode(),
            aspect_mask: cube_map_int.aspect_flags,
            array_elem: copy.cube_map_array_element as u32,
            mip_level: copy.cube_map_mip_level as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_blit(
    state: &mut TrackState,
    src: &BlitSource<crate::VulkanBackend>,
    dst: &BlitDestination<crate::VulkanBackend>,
    blit: &Blit,
) {
    // Barrier check
    let (src_img, src_array_elem, src_aspect_flags, src_queue_types, src_sharing_mode) = match src {
        BlitSource::Texture(tex) => {
            let internal = tex.internal();
            (
                internal.image,
                blit.src_array_element,
                internal.aspect_flags,
                tex.queue_types(),
                tex.sharing_mode(),
            )
        }
        BlitSource::CubeMap { cube_map, face } => {
            let internal = cube_map.internal();
            (
                internal.image,
                crate::cube_map::CubeMap::to_array_elem(blit.src_array_element, *face),
                internal.aspect_flags,
                cube_map.queue_types(),
                cube_map.sharing_mode(),
            )
        }
    };

    let (dst_img, dst_array_elem, dst_aspect_flags, dst_queue_types, dst_sharing_mode) = match dst {
        BlitDestination::Texture(tex) => {
            let internal = tex.internal();
            (
                internal.image,
                blit.dst_array_element,
                internal.aspect_flags,
                tex.queue_types(),
                tex.sharing_mode(),
            )
        }
        BlitDestination::CubeMap { cube_map, face } => {
            let internal = cube_map.internal();
            (
                internal.image,
                crate::cube_map::CubeMap::to_array_elem(blit.dst_array_element, *face),
                internal.aspect_flags,
                cube_map.queue_types(),
                cube_map.sharing_mode(),
            )
        }
        BlitDestination::SurfaceImage(si) => {
            let internal = si.internal();
            let semaphores = internal.semaphores();

            // Also handle semaphores of the surface image
            state
                .semaphores
                .register_signal(semaphores.presentable, None);
            state.semaphores.register_wait(
                semaphores.available,
                WaitInfo {
                    value: None,
                    stage: vk::PipelineStageFlags::TRANSFER,
                },
            );

            (
                internal.image(),
                0,
                vk::ImageAspectFlags::COLOR,
                QueueTypes::all(),
                SharingMode::Concurrent,
            )
        }
    };

    let mut scope = UsageScope::default();
    scope.use_resource(
        SubResource::Texture {
            texture: src_img,
            queue_types: src_queue_types,
            sharing: src_sharing_mode,
            aspect_mask: src_aspect_flags,
            array_elem: src_array_elem as u32,
            mip_level: blit.src_mip as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_READ,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
        },
    );
    scope.use_resource(
        SubResource::Texture {
            texture: dst_img,
            queue_types: dst_queue_types,
            sharing: dst_sharing_mode,
            aspect_mask: dst_aspect_flags,
            array_elem: dst_array_elem as u32,
            mip_level: blit.dst_mip as u32,
        },
        SubResourceUsage {
            access: vk::AccessFlags::TRANSFER_WRITE,
            stage: vk::PipelineStageFlags::TRANSFER,
            layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
        },
    );

    if let Some(barrier) = state.pipeline_tracker.submit(scope) {
        barrier.execute(state.device, state.command_buffer);
    }
}

unsafe fn track_descriptor_sets(
    sets: &[&DescriptorSet<crate::VulkanBackend>],
    set_stage: vk::PipelineStageFlags,
    scope: &mut UsageScope,
) {
    for set in sets {
        track_descriptor_set(set, set_stage, scope);
    }
}

unsafe fn track_descriptor_set(
    set: &DescriptorSet<crate::VulkanBackend>,
    set_stage: vk::PipelineStageFlags,
    scope: &mut UsageScope,
) {
    scope.use_resource(
        SubResource::Set {
            set: set.internal().set,
        },
        SubResourceUsage {
            access: vk::AccessFlags::empty(),
            stage: set_stage,
            layout: vk::ImageLayout::UNDEFINED,
        },
    );

    // Check every binding of every set
    for binding in &set.internal().bound {
        // Check every element of every binding
        for elem in binding.iter().flatten() {
            // Only care about elements if they are filled
            match &elem.value {
                BoundValue::UniformBuffer {
                    buffer,
                    array_element,
                    queue_types,
                    sharing_mode,
                    aligned_size,
                    ..
                } => scope.use_resource(
                    SubResource::Buffer {
                        buffer: *buffer,
                        queue_types: *queue_types,
                        sharing: *sharing_mode,
                        array_elem: *array_element as u32,
                        aligned_size: *aligned_size,
                    },
                    SubResourceUsage {
                        access: elem.access,
                        stage: elem.stage,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                ),
                BoundValue::StorageBuffer {
                    buffer,
                    array_element,
                    queue_types,
                    sharing_mode,
                    aligned_size,
                    ..
                } => scope.use_resource(
                    SubResource::Buffer {
                        buffer: *buffer,
                        queue_types: *queue_types,
                        sharing: *sharing_mode,
                        array_elem: *array_element as u32,
                        aligned_size: *aligned_size,
                    },
                    SubResourceUsage {
                        access: elem.access,
                        stage: elem.stage,
                        layout: vk::ImageLayout::UNDEFINED,
                    },
                ),
                // Textures require that you register each mip individually
                BoundValue::Texture {
                    _ref_counter,
                    image,
                    array_element,
                    aspect_mask,
                    mip_count,
                    base_mip,
                    queue_types,
                    sharing_mode,
                    ..
                } => {
                    for i in 0..*mip_count {
                        scope.use_resource(
                            SubResource::Texture {
                                texture: *image,
                                aspect_mask: *aspect_mask,
                                array_elem: *array_element as u32,
                                mip_level: base_mip + i,
                                queue_types: *queue_types,
                                sharing: *sharing_mode,
                            },
                            SubResourceUsage {
                                access: elem.access,
                                stage: elem.stage,
                                layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                            },
                        )
                    }
                }
                BoundValue::StorageImage {
                    _ref_counter,
                    image,
                    aspect_mask,
                    mip,
                    array_element,
                    queue_types,
                    sharing_mode,
                    ..
                } => scope.use_resource(
                    SubResource::Texture {
                        texture: *image,
                        aspect_mask: *aspect_mask,
                        array_elem: *array_element as u32,
                        mip_level: *mip,
                        queue_types: *queue_types,
                        sharing: *sharing_mode,
                    },
                    SubResourceUsage {
                        access: elem.access,
                        stage: elem.stage,
                        layout: vk::ImageLayout::GENERAL,
                    },
                ),
                // Cube maps require that you register each mip individually
                BoundValue::CubeMap {
                    image,
                    aspect_mask,
                    mip_count,
                    base_mip,
                    array_element,
                    queue_types,
                    sharing_mode,
                    ..
                } => {
                    for i in 0..*mip_count {
                        scope.use_resource(
                            SubResource::CubeMap {
                                cube_map: *image,
                                aspect_mask: *aspect_mask,
                                array_elem: *array_element as u32,
                                queue_types: *queue_types,
                                sharing: *sharing_mode,
                                mip_level: base_mip + i,
                            },
                            SubResourceUsage {
                                access: elem.access,
                                stage: elem.stage,
                                layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                            },
                        )
                    }
                }
            }
        }
    }
}
