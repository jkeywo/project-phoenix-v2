//! Small depth-occluded lens flare. Based on Bevy 0.18's custom render-pass API.
use bevy::{
    core_pipeline::{
        core_3d::graph::{Core3d, Node3d},
        prepass::ViewPrepassTextures,
        FullscreenShader,
    },
    ecs::query::QueryItem,
    prelude::*,
    render::{
        extract_component::{
            ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
            UniformComponentPlugin,
        },
        render_graph::{
            NodeRunError, RenderGraphContext, RenderGraphExt, RenderLabel, ViewNode, ViewNodeRunner,
        },
        render_resource::{
            binding_types::{
                sampler, texture_2d, texture_depth_2d, texture_depth_2d_multisampled,
                uniform_buffer,
            },
            *,
        },
        renderer::{RenderContext, RenderDevice},
        view::ViewTarget,
        RenderApp, RenderStartup,
    },
};

#[derive(Component, Default, Clone, Copy, ExtractComponent, ShaderType)]
pub struct FlareSettings {
    // xy: source UV; z: reverse-Z depth of source's FRONT; w: strength.
    pub source: Vec4,
    // xy: projected source radius; z: viewport aspect; w: reserved.
    pub shape: Vec4,
}

pub struct FlarePlugin;
impl Plugin for FlarePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            ExtractComponentPlugin::<FlareSettings>::default(),
            UniformComponentPlugin::<FlareSettings>::default(),
        ));
        let Some(render) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render
            .add_systems(RenderStartup, prepare)
            .add_render_graph_node::<ViewNodeRunner<FlareNode>>(Core3d, FlareLabel)
            .add_render_graph_edges(
                Core3d,
                (
                    Node3d::Tonemapping,
                    FlareLabel,
                    Node3d::EndMainPassPostProcessing,
                ),
            );
    }
}
#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct FlareLabel;
#[derive(Default)]
struct FlareNode;
impl ViewNode for FlareNode {
    type ViewQuery = (
        &'static ViewTarget,
        &'static Msaa,
        &'static ViewPrepassTextures,
        &'static FlareSettings,
        &'static DynamicUniformIndex<FlareSettings>,
    );
    fn run(
        &self,
        _: &mut RenderGraphContext,
        context: &mut RenderContext,
        (target, msaa, prepass, settings, index): QueryItem<Self::ViewQuery>,
        world: &World,
    ) -> Result<(), NodeRunError> {
        if settings.source.w <= 0.0 {
            return Ok(());
        }
        let state = world.resource::<FlarePipeline>();
        let cache = world.resource::<PipelineCache>();
        let variant = usize::from(msaa.samples() > 1);
        let id = state.pipelines[variant * 2 + usize::from(target.is_hdr())];
        let Some(pipeline) = cache.get_render_pipeline(id) else {
            return Ok(());
        };
        let Some(depth) = prepass.depth_view() else {
            return Ok(());
        };
        let uniforms = world.resource::<ComponentUniforms<FlareSettings>>();
        let Some(binding) = uniforms.uniforms().binding() else {
            return Ok(());
        };
        let post = target.post_process_write();
        let group = context.render_device().create_bind_group(
            "star_flare",
            &cache.get_bind_group_layout(&state.layouts[variant]),
            &BindGroupEntries::sequential((post.source, &state.sampler, binding, depth)),
        );
        let mut pass = context.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("star_flare"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: post.destination,
                depth_slice: None,
                resolve_target: None,
                ops: Operations::default(),
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_render_pipeline(pipeline);
        pass.set_bind_group(0, &group, &[index.index()]);
        pass.draw(0..3, 0..1);
        Ok(())
    }
}
#[derive(Resource)]
struct FlarePipeline {
    layouts: Vec<BindGroupLayoutDescriptor>,
    sampler: Sampler,
    pipelines: Vec<CachedRenderPipelineId>,
}
fn prepare(
    mut commands: Commands,
    device: Res<RenderDevice>,
    assets: Res<AssetServer>,
    fullscreen: Res<FullscreenShader>,
    cache: Res<PipelineCache>,
) {
    let shader = assets.load("shaders/star_flare.wgsl");
    let mut layouts = Vec::new();
    let mut pipelines = Vec::new();
    for multisampled in [false, true] {
        let depth = if multisampled {
            texture_depth_2d_multisampled()
        } else {
            texture_depth_2d()
        };
        let layout = BindGroupLayoutDescriptor::new(
            "star_flare",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<FlareSettings>(true),
                    depth,
                ),
            ),
        );
        for format in [
            TextureFormat::bevy_default(),
            ViewTarget::TEXTURE_FORMAT_HDR,
        ] {
            pipelines.push(cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("star_flare".into()),
                layout: vec![layout.clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: shader.clone(),
                    shader_defs: if multisampled {
                        vec!["MULTISAMPLED_DEPTH".into()]
                    } else {
                        vec![]
                    },
                    targets: vec![Some(ColorTargetState {
                        format,
                        blend: None,
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            }));
        }
        layouts.push(layout);
    }
    commands.insert_resource(FlarePipeline {
        layouts,
        pipelines,
        sampler: device.create_sampler(&SamplerDescriptor::default()),
    });
}
