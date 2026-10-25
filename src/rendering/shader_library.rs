//! Named shaders - what a node's `Material` (see scene_manager::node::properties)
//! draws its model with.
//!
//! - Built in, under `engine/`: "engine/standard" (what every model is drawn
//!   with unless it says otherwise), "engine/error" (see below) and
//!   "engine/blit" (a Fullscreen shader copying the scene onto the screen).
//! - The game's, by path name: `add_folder("assets/shaders")` makes
//!   `assets/shaders/hologram.wgsl` "hologram" and `.../ui/glitch.wgsl`
//!   "ui/glitch"; `add` registers one from source (built into the game),
//!   `add_file` one file under a name of its own.
//! - `override_builtin` swaps a built-in for one of the game's (every model
//!   without a material of its own, drawn some other way).
//!
//! A shader's settings are in a `.ron` next to its `.wgsl` (see
//! `ShaderSettings` - no `.ron`, the defaults). A `Surface` shader is only
//! hooks - `vertex()` and/or `fragment()` - pasted into the engine's
//! template (standard_template.wgsl), which gives them the camera, the
//! mesh, the light and `engine.*`; a `Raw` one is a whole shader taking the
//! same inputs (bind groups 0-3, the model vertex and instance layouts).
//!
//! Its own values: a `struct Params` in a Surface shader becomes `params`
//! (filled from its `.ron`'s `params`, then the material's own - see
//! `Material::param` - by field name: a number, or a list for a vector or an
//! array), and each of the `.ron`'s `textures` a `texture_2d` of that name
//! with a sampler `<name>_sampler`. They share bind group 0 with the mesh's
//! own texture (there are only four groups): params at binding 2, the
//! textures from 3 on, two bindings each (texture, sampler), by name in
//! alphabetical order - what a Raw shader declares itself. One that `reads:
//! [Depth]` gets the depth so far (the frame plan's CopyDepth) at 16 - a
//! `texture_2d<f32>` to `textureLoad` - and a non-filtering sampler at 17;
//! a Surface shader declares them itself, and turns a depth into a distance
//! with the template's `linear_depth` (the camera's planes are `engine.near`
//! and `engine.far`).
//!
//! A `Fullscreen` shader is a frame plan's `Fullscreen` step - a `fragment()`
//! hook run for every pixel of the target (fullscreen_template.wgsl). It reads
//! the images its `.ron` names in `inputs` ("scene" - the world drawn so far,
//! "blurred" - the scene blurred, last frame's - or one of the game's own
//! targets, by its name: see Renderer::add_target),
//! each declared for it like a texture, after its textures; its params are
//! the material `key`'s the step names (see `set_instance`). One that can't
//! be used falls back to engine/blit (the scene, plain) - said once.
//!
//! A `Ui`, `Text` or `Lines` shader is a `fragment()` hook for the engine's 2D
//! drawing - UI boxes and images, SDF text sets, thick line sets - each with
//! its own template (ui_template.wgsl, ui_image_template.wgsl,
//! sdf_text_template.wgsl, thick_lines_template.wgsl). The engine works out
//! each pixel as it always does and hands the hook its `color` (with a few
//! things about where it is - see each template's `UiIn`, `TextIn`,
//! `LineIn`); the hook returns what's drawn. The engine's own are
//! "engine/ui", "engine/sdf_text" and "engine/thick_lines" - the hook alone,
//! returning `color`. Their params and textures are in group 0 like a
//! Surface shader's (no mesh texture); group 1 is the camera, 2 what's being
//! drawn, 3 the light (`engine.*`). One that can't be used draws in flashing
//! magenta - same shape, so it's plain where it is - said once.
//!
//! A node's material with values of its own is its own material instance
//! (`set_instance`, keyed by node) - its own copy of the params, rewritten
//! when they change; one without shares the shader's.
//!
//! A shader is read and built the first time a material asks for it -
//! checked with naga first, so a mistake comes back with the line in the
//! shader's own file, then built inside a GPU error scope. One that's
//! missing or doesn't work (or is the wrong kind, or wants a pass the frame
//! plan doesn't have) draws with "engine/error" - flashing magenta - and
//! says why in the console, once. Its files are watched: saved, it's built
//! again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};

use crate::rendering::instance_management::InstanceRaw;
use crate::rendering::models::model::{self, Vertex};
use crate::rendering::models::textures::Texture;

pub const STANDARD: &str = "engine/standard";
pub const ERROR: &str = "engine/error";
/// The Fullscreen shader that copies the scene onto the target, untouched.
pub const BLIT: &str = "engine/blit";
/// What UI boxes and images are drawn with unless they say otherwise.
pub const UI: &str = "engine/ui";
/// What SDF text sets are drawn with unless they say otherwise.
pub const SDF_TEXT: &str = "engine/sdf_text";
/// What thick line sets are drawn with unless they say otherwise.
pub const THICK_LINES: &str = "engine/thick_lines";
/// The engine's own images a Fullscreen shader can read (its `.ron`'s
/// `inputs`) - besides the game's targets (see Renderer::add_target).
pub const BUILTIN_SCREEN_INPUTS: [&str; 2] = ["scene", "blurred"];
const FULLSCREEN_TEMPLATE: &str = include_str!("../shaders/fullscreen_template.wgsl");
const BLIT_HOOKS: &str = "fn fragment(f: ScreenIn) -> vec4<f32> {\n    return textureSample(scene, scene_sampler, f.uv);\n}\n";
/// Where a shader that `reads: [Depth]` gets the depth so far - texture,
/// then sampler - in its bind group 0.
pub const DEPTH_BINDING: u32 = 16;

const TEMPLATE: &str = include_str!("../shaders/standard_template.wgsl");
const UI_TEMPLATE: &str = include_str!("../shaders/ui_template.wgsl");
const UI_IMAGE_TEMPLATE: &str = include_str!("../shaders/ui_image_template.wgsl");
const SDF_TEXT_TEMPLATE: &str = include_str!("../shaders/sdf_text_template.wgsl");
const THICK_LINES_TEMPLATE: &str = include_str!("../shaders/thick_lines_template.wgsl");
const ERROR_SOURCE: &str = include_str!("../shaders/error.wgsl");
/// Where a Surface shader's hooks go in the template.
const HOOKS_MARKER: &str = "//@HOOKS";
const DEFAULT_VERTEX: &str = "fn vertex(v: VertexIn) -> VertexIn {\n    return v;\n}\n";
const DEFAULT_FRAGMENT: &str = "fn fragment(f: FragmentIn) -> vec4<f32> {\n    return f.lit_color;\n}\n";
/// How often the shaders' files are checked for changes.
const WATCH_INTERVAL: Duration = Duration::from_secs(1);

/// What a shader is - which template it goes in, and what it can be used on.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    /// Hooks for 3D models (see the module doc).
    #[default]
    Surface,
    /// A `fragment()` hook for UI boxes and images (see the module doc).
    Ui,
    /// A `fragment()` hook for SDF text sets.
    Text,
    /// A `fragment()` hook for thick line sets.
    Lines,
    /// A whole shader, taking the engine's standard inputs.
    Raw,
    /// A `fragment()` hook run for every pixel of a target - a frame plan's
    /// `Fullscreen` step (see the module doc).
    Fullscreen,
}

impl Kind {
    /// The 2D things a shader of this kind is built for - none for a 3D or
    /// Fullscreen one.
    pub fn overlays(self) -> &'static [Overlay] {
        match self {
            Kind::Ui => &[Overlay::UiBox, Overlay::UiImage],
            Kind::Text => &[Overlay::Text],
            Kind::Lines => &[Overlay::Lines],
            _ => &[],
        }
    }

    /// What a 2D thing of this kind is drawn with when its own shader can't
    /// be used - the engine's, in flashing magenta.
    fn error_shader(self) -> &'static str {
        match self {
            Kind::Text => "engine/sdf_text_error",
            Kind::Lines => "engine/thick_lines_error",
            _ => "engine/ui_error",
        }
    }

    /// Its hook's input - see the 2D templates.
    fn input(self) -> &'static str {
        match self {
            Kind::Text => "TextIn",
            Kind::Lines => "LineIn",
            _ => "UiIn",
        }
    }
}

/// One of the engine's 2D things a Ui, Text or Lines shader draws - each its
/// own template, vertices and bind group 2 (see the module doc).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overlay {
    /// A UI node's box - panel, button, a label's background.
    UiBox,
    /// A UI image node.
    UiImage,
    /// An SDF text set's glyphs.
    Text,
    /// A thick line set's segments.
    Lines,
}

impl Overlay {
    const ALL: [Overlay; 4] = [Overlay::UiBox, Overlay::UiImage, Overlay::Text, Overlay::Lines];

    fn template(self) -> &'static str {
        match self {
            Overlay::UiBox => UI_TEMPLATE,
            Overlay::UiImage => UI_IMAGE_TEMPLATE,
            Overlay::Text => SDF_TEXT_TEMPLATE,
            Overlay::Lines => THICK_LINES_TEMPLATE,
        }
    }

    fn vertex_layout(self) -> wgpu::VertexBufferLayout<'static> {
        use crate::rendering::vertex::{ImageVertex, VertexUi};
        match self {
            Overlay::UiBox => VertexUi::desc(),
            Overlay::UiImage => ImageVertex::desc(),
            Overlay::Text => crate::rendering::sdf_text::glyph_layout(),
            Overlay::Lines => crate::rendering::thick_lines::line_layout(),
        }
    }

    /// UI quads are always wound the same way; a glyph or a line's quad can
    /// face either way once projected.
    fn cull(self) -> Option<wgpu::Face> {
        match self {
            Overlay::UiBox | Overlay::UiImage => Some(wgpu::Face::Back),
            Overlay::Text | Overlay::Lines => None,
        }
    }

    /// Its bind group 2's layout - what the renderer drawing it binds there.
    fn bind_group_layout(self, device: &wgpu::Device) -> wgpu::BindGroupLayout {
        match self {
            Overlay::UiBox => crate::rendering::ui::ui::Ui::box_bind_group_layout(device),
            Overlay::UiImage => Texture::create_bind_group_layout(device),
            Overlay::Text => crate::rendering::sdf_text::bind_group_layout(device),
            Overlay::Lines => crate::rendering::thick_lines::bind_group_layout(device),
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Blend {
    /// Over what's behind, by its alpha.
    Alpha,
    /// Added to what's behind (glows, fire).
    Additive,
    /// Replaces what's behind.
    #[default]
    None,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Cull {
    /// Back faces aren't drawn.
    #[default]
    Back,
    Front,
    /// Both sides are.
    None,
}

/// Something a shader reads beyond the standard inputs.
#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    /// The depth so far (a frame plan's CopyDepth before its pass).
    Depth,
}

/// A shader's `.ron` - everything about it but its code. Every field is
/// optional; a misspelled one is an error.
#[derive(Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields, default)]
pub struct ShaderSettings {
    pub kind: Kind,
    /// The frame plan pass it draws in, by name. Left out: like
    /// engine/standard - a model's solid meshes in "opaque", its see-through
    /// ones in "transparent".
    pub pass: Option<String>,
    /// Left out: none for a 3D or Fullscreen shader, Alpha for a 2D one (see
    /// `blend()`).
    pub blend: Option<Blend>,
    pub cull: Cull,
    pub depth_write: bool,
    /// Defaults for its `Params` fields, by name - a number, or a list (a
    /// vector, an array).
    pub params: BTreeMap<String, ParamValue>,
    /// Textures it reads, by name - an image file each.
    pub textures: BTreeMap<String, String>,
    pub reads: Vec<Read>,
    /// A Fullscreen shader's images, by name - see BUILTIN_SCREEN_INPUTS.
    pub inputs: Vec<String>,
}

impl Default for ShaderSettings {
    fn default() -> Self {
        Self {
            kind: Kind::Surface,
            pass: None,
            blend: None,
            cull: Cull::Back,
            depth_write: true,
            params: BTreeMap::new(),
            textures: BTreeMap::new(),
            reads: Vec::new(),
            inputs: Vec::new(),
        }
    }
}

impl ShaderSettings {
    /// How it blends - its `blend`, else its kind's: over what's behind for
    /// a 2D one (UI, text and lines all are), replacing it otherwise.
    pub fn blend(&self) -> Blend {
        self.blend.unwrap_or(if self.kind.overlays().is_empty() { Blend::None } else { Blend::Alpha })
    }
}

/// A parameter's value - a number, or numbers for a vector or an array (an
/// array of vectors flattened: `(x, y, z, w, x, y, z, w...)`). In a `.ron`:
/// `1.5`, `(0.2, 0.8, 1.0)`, `[(1, 0, 0, 1), (0, 1, 0, 1)]`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ParamValue(pub Vec<f32>);

impl From<f32> for ParamValue {
    fn from(value: f32) -> Self {
        Self(vec![value])
    }
}

impl<const N: usize> From<[f32; N]> for ParamValue {
    fn from(values: [f32; N]) -> Self {
        Self(values.to_vec())
    }
}

impl From<Vec<f32>> for ParamValue {
    fn from(values: Vec<f32>) -> Self {
        Self(values)
    }
}

impl<'de> Deserialize<'de> for ParamValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = ron::Value::deserialize(deserializer)?;
        floats(&value).map(ParamValue).ok_or_else(|| serde::de::Error::custom("a parameter is a number, or a list of numbers"))
    }
}

impl Serialize for ParamValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.as_slice() {
            [one] => serializer.serialize_f32(*one),
            many => many.serialize(serializer),
        }
    }
}

/// The numbers in a `.ron` value - a number, true/false (1/0), or lists of
/// them (flattened). None for anything else.
pub fn floats(value: &ron::Value) -> Option<Vec<f32>> {
    match value {
        ron::Value::Number(number) => Some(vec![number.into_f64() as f32]),
        ron::Value::Bool(flag) => Some(vec![if *flag { 1.0 } else { 0.0 }]),
        ron::Value::Option(Some(inner)) => floats(inner),
        ron::Value::Seq(items) => items.iter().map(floats).collect::<Option<Vec<_>>>().map(|lists| lists.concat()),
        _ => None,
    }
}

/// One field of a shader's `Params` - where it is in the block, what it
/// holds (how many numbers, of which kind), and for an array, how many and
/// how far apart.
#[derive(Clone, Debug, PartialEq)]
struct ParamField {
    name: String,
    offset: u32,
    kind: wgpu::naga::ScalarKind,
    components: u32,
    array: Option<(u32, u32)>,
}

/// A shader's `Params` block - its size and fields (see `params_layout`).
#[derive(Clone, Debug, PartialEq)]
struct ParamsLayout {
    size: u64,
    fields: Vec<ParamField>,
}

/// One of a shader's pipelines: which faces it culls, whether it writes
/// depth, how it blends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PipelineKey {
    pub cull: Cull,
    pub depth_write: bool,
    pub blend: Blend,
}

/// Where a material's models are drawn this frame - see `ShaderLibrary::
/// placement`.
#[derive(Clone, Debug, PartialEq)]
pub enum Placement {
    /// Like engine/standard: solid meshes in "opaque", see-through ones in
    /// "transparent".
    ByMesh,
    /// Every mesh in this pass.
    Pass(String),
    /// With engine/error, in "opaque".
    Error,
}

impl Placement {
    /// The pipelines a material placed like this draws with.
    pub fn keys(&self, settings: &ShaderSettings) -> Vec<PipelineKey> {
        match self {
            // The solid list (cull or not), then the see-through list (cull
            // back, or front then back for a double-sided shell) - the same
            // four the engine always drew models with.
            Placement::ByMesh => vec![
                PipelineKey { cull: Cull::Back, depth_write: true, blend: Blend::Alpha },
                PipelineKey { cull: Cull::None, depth_write: true, blend: Blend::Alpha },
                PipelineKey { cull: Cull::Back, depth_write: false, blend: Blend::Alpha },
                PipelineKey { cull: Cull::Front, depth_write: false, blend: Blend::Alpha },
            ],
            Placement::Pass(_) => vec![
                PipelineKey { cull: settings.cull, depth_write: settings.depth_write, blend: settings.blend() },
                PipelineKey { cull: Cull::None, depth_write: settings.depth_write, blend: settings.blend() },
            ],
            Placement::Error => vec![
                PipelineKey { cull: Cull::Back, depth_write: true, blend: Blend::None },
                PipelineKey { cull: Cull::None, depth_write: true, blend: Blend::None },
            ],
        }
    }
}

/// Where a shader's code and settings come from.
#[derive(Clone)]
enum Source {
    /// engine/standard - the template with the default hooks.
    Standard,
    /// engine/error.
    Error,
    /// engine/blit.
    Blit,
    /// The engine's own 2D shader of a kind (engine/ui, engine/sdf_text,
    /// engine/thick_lines) - or, `error`, its flashing magenta one.
    Overlay { kind: Kind, error: bool },
    /// Registered with `add`: its code, and its settings' text.
    Embedded { wgsl: String, settings: Option<String> },
    /// A `.wgsl` on disk, with its `.ron` (if there is one) next to it.
    File { wgsl: PathBuf },
}

/// Files, and when each was last changed (None: not there).
type Watch = Vec<(PathBuf, Option<SystemTime>)>;

struct Shader {
    settings: ShaderSettings,
    module: Option<wgpu::ShaderModule>,
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    /// A 2D shader's pipeline for each thing it draws.
    overlays: HashMap<Overlay, wgpu::RenderPipeline>,
    /// Its files - built again when one changes.
    watch: Watch,
    /// Its own bind group 0, when it has params or textures - otherwise it
    /// takes the mesh's own (just its texture).
    group0: Option<Group0>,
    /// Which build this is - a material instance made for an older one
    /// makes its buffer and bind groups again.
    generation: u64,
}

/// A shader's own bind group 0: the mesh's texture, its params block, its
/// textures, the depth so far (see the module doc).
struct Group0 {
    layout: wgpu::BindGroupLayout,
    /// A 3D or Fullscreen shader's - a 2D one has one per thing it draws.
    pipeline_layout: Option<wgpu::PipelineLayout>,
    params: Option<ParamsLayout>,
    textures: Vec<Texture>,
    depth: bool,
    /// A Fullscreen shader's inputs, after its textures.
    inputs: Vec<String>,
}

/// A material in use - a shader, and (for a node's own) the values it sets.
struct Instance {
    shader: String,
    params: BTreeMap<String, ParamValue>,
    /// Its params block, as last written, for the shader build it's for.
    buffer: Option<(wgpu::Buffer, Vec<u8>)>,
    /// Its bind group 0 for each mesh texture it's drawn with - by the
    /// model it's in, then the texture's index there (a 2D or Fullscreen
    /// material's one: model "", index 0). Nested so looking one up needs no
    /// new String - it's done for every mesh, every frame.
    bind_groups: HashMap<String, HashMap<usize, wgpu::BindGroup>>,
    generation: u64,
}

/// Why a shader couldn't be built - and its files as they were then, so
/// fixing one (or making a missing one) builds it again.
#[derive(Debug)]
struct Failure {
    problem: String,
    watch: Watch,
}

pub struct ShaderLibrary {
    device: wgpu::Device,
    queue: wgpu::Queue,
    color_format: wgpu::TextureFormat,
    /// The model pipelines' bind groups' layouts - the mesh's texture, the
    /// camera, the mesh's transform, the light - and the pipeline layout of
    /// all four, for shaders with no group 0 of their own.
    groups: [wgpu::BindGroupLayout; 4],
    layout: wgpu::PipelineLayout,
    /// Bind group 2 of each 2D thing - see `overlay_layout`.
    overlay_layouts: HashMap<Overlay, wgpu::BindGroupLayout>,
    instances: HashMap<String, Instance>,
    /// Goes up whenever what's built changes - a shader built or forgotten,
    /// the images bind groups are made with made anew - see `changes`.
    changes: u64,
    generation: u64,
    /// The depth snapshot (DepthRender::foam_depth_copy) - what a shader that
    /// `reads: [Depth]` gets. Set at startup and again on every resize.
    depth_copy: Option<(wgpu::TextureView, wgpu::Sampler)>,
    /// The images Fullscreen shaders read, by name (see BUILTIN_SCREEN_INPUTS
    /// and Renderer::add_target) - set
    /// at startup and again on every resize.
    screen_inputs: HashMap<String, (wgpu::TextureView, wgpu::Sampler)>,
    folders: Vec<PathBuf>,
    registered: HashMap<String, Source>,
    overrides: HashMap<String, String>,
    /// Each shader asked for so far - built, or why it couldn't be.
    built: HashMap<String, Result<Shader, Failure>>,
    /// Problems already said, so each is said once.
    reported: HashSet<String>,
    last_watch: Instant,
}

impl ShaderLibrary {
    /// For models drawn into `color_format` with the model pipelines' bind
    /// groups (`groups`: the mesh's texture, the camera, the mesh's
    /// transform, the light).
    pub fn new(device: wgpu::Device, queue: wgpu::Queue, color_format: wgpu::TextureFormat, groups: [wgpu::BindGroupLayout; 4]) -> Self {
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Model pipeline layout"),
            bind_group_layouts: &[&groups[0], &groups[1], &groups[2], &groups[3]],
            push_constant_ranges: &[],
        });
        let overlay_layouts = Overlay::ALL.into_iter().map(|overlay| (overlay, overlay.bind_group_layout(&device))).collect();
        Self {
            device,
            queue,
            color_format,
            groups,
            layout,
            overlay_layouts,
            instances: HashMap::new(),
            changes: 0,
            generation: 0,
            depth_copy: None,
            screen_inputs: HashMap::new(),
            folders: Vec::new(),
            registered: HashMap::new(),
            overrides: HashMap::new(),
            built: HashMap::new(),
            reported: HashSet::new(),
            last_watch: Instant::now(),
        }
    }

    /// The depth snapshot shaders that `reads: [Depth]` get - set at startup
    /// and again whenever it's made anew (a resize): every bind group made
    /// with the old one goes.
    pub fn set_depth_copy(&mut self, view: wgpu::TextureView, sampler: wgpu::Sampler) {
        self.changes += 1;
        self.depth_copy = Some((view, sampler));
        for instance in self.instances.values_mut() {
            instance.bind_groups.clear();
        }
    }

    /// The images Fullscreen shaders read (see `screen_inputs`) - set at
    /// startup and again whenever they're made anew (a resize, a target
    /// added): every bind group made with the old ones goes.
    pub fn set_screen_inputs(&mut self, inputs: Vec<(&str, wgpu::TextureView, wgpu::Sampler)>) {
        self.changes += 1;
        self.screen_inputs = inputs.into_iter().map(|(name, view, sampler)| (name.to_owned(), (view, sampler))).collect();
        for instance in self.instances.values_mut() {
            instance.bind_groups.clear();
        }
    }

    /// Gets the material `key` (a Fullscreen shader's name, or an instance of
    /// one - see `set_instance`) ready for a frame plan's Fullscreen step:
    /// its shader, its pipeline, its params, its bind group. False (said
    /// once) when it can't be used - the step draws engine/blit instead.
    pub fn prepare_fullscreen(&mut self, key: &str) -> bool {
        let name = self.shader_of(key);
        let mut images: Vec<String> = self.screen_inputs.keys().cloned().collect();
        images.sort();
        let problem = match self.ensure_built(&name) {
            Err(problem) => Some(problem),
            Ok(settings) if settings.kind != Kind::Fullscreen => Some(format!("it's a {:?} shader, used as a Fullscreen step", settings.kind)),
            Ok(settings) => settings.inputs.iter().find(|input| !images.contains(input)).map(|input| format!("its input \"{input}\" isn't an image there is ({})", images.join(", "))),
        };
        if let Some(problem) = problem {
            self.report_fullscreen(&name, &problem);
            return false;
        }
        if let Err(problem) = self.ensure_fullscreen_pipeline(&name) {
            self.report_fullscreen(&name, &problem);
            return false;
        }
        self.prepare_material(key)
    }

    /// Writes the material `key`'s params and makes its bind group 0 (params,
    /// textures, inputs - no mesh texture) if it isn't made yet - for a
    /// Fullscreen or 2D shader, once built. False if it can't be.
    fn prepare_material(&mut self, key: &str) -> bool {
        let name = self.shader_of(key);
        if self.ensure_built(&name).is_err() {
            return false;
        }
        self.write_params(key);
        let Some(Ok(shader)) = self.built.get(&name) else { return false };
        let Some(group0) = &shader.group0 else { return false };
        let Some(instance) = self.instances.get_mut(key) else { return false };
        if !instance.bind_groups.get("").is_some_and(|groups| groups.contains_key(&0)) {
            let mut entries = Vec::new();
            if group0.params.is_some() {
                let Some((buffer, _)) = &instance.buffer else { return false };
                entries.push(wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() });
            }
            for (index, texture) in group0.textures.iter().enumerate() {
                let binding = 3 + 2 * index as u32;
                entries.push(wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(&texture.view) });
                entries.push(wgpu::BindGroupEntry { binding: binding + 1, resource: wgpu::BindingResource::Sampler(&texture.sampler) });
            }
            for (index, input) in group0.inputs.iter().enumerate() {
                let Some((view, sampler)) = self.screen_inputs.get(input) else { return false };
                let binding = 3 + 2 * (group0.textures.len() + index) as u32;
                entries.push(wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) });
                entries.push(wgpu::BindGroupEntry { binding: binding + 1, resource: wgpu::BindingResource::Sampler(sampler) });
            }
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(key), layout: &group0.layout, entries: &entries });
            instance.bind_groups.entry(String::new()).or_default().insert(0, bind_group);
        }
        true
    }

    /// A count that goes up whenever what's built changes (a shader built
    /// or forgotten, the depth copy or the screen images made anew) - the
    /// same two frames in a row, nothing here changed in between.
    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// The material `key`'s Fullscreen pipeline and bind group 0, once
    /// `prepare_fullscreen` has made them.
    pub fn fullscreen(&self, key: &str) -> Option<(&wgpu::RenderPipeline, &wgpu::BindGroup)> {
        let shader = self.built.get(self.shader_name(key))?.as_ref().ok()?;
        let pipeline = shader.pipelines.get(&Self::fullscreen_key(&shader.settings))?;
        let bind_group = self.instances.get(key)?.bind_groups.get("")?.get(&0)?;
        Some((pipeline, bind_group))
    }

    /// Gets the material `key` ready to draw `kind`'s 2D things with (see the
    /// module doc) - its shader built, its params written, its bind group
    /// made. What to draw with: `key` - or, when it can't be used (said once),
    /// the kind's flashing magenta one. None if not even that can be.
    pub fn prepare_overlay(&mut self, key: &str, kind: Kind) -> Option<String> {
        let name = self.shader_of(key);
        let problem = match self.ensure_built(&name) {
            Err(problem) => Some(problem),
            Ok(settings) if settings.kind != kind => Some(format!("it's a {:?} shader, used as a {kind:?} one", settings.kind)),
            Ok(_) => None,
        };
        if problem.is_none() && self.prepare_material(key) {
            return Some(key.to_owned());
        }
        let problem = problem.unwrap_or_else(|| "its bind group couldn't be made".to_owned());
        self.report(&name, &problem);
        let error = kind.error_shader();
        (key != error && self.prepare_material(error)).then(|| error.to_owned())
    }

    /// The material `key`'s pipeline for `overlay` and its bind group 0, once
    /// `prepare_overlay` has made them.
    pub fn overlay(&self, key: &str, overlay: Overlay) -> Option<(&wgpu::RenderPipeline, &wgpu::BindGroup)> {
        let shader = self.built.get(self.shader_name(key))?.as_ref().ok()?;
        let pipeline = shader.overlays.get(&overlay)?;
        let bind_group = self.instances.get(key)?.bind_groups.get("")?.get(&0)?;
        Some((pipeline, bind_group))
    }

    /// What `overlay`'s renderer binds as group 2 is made with (see the module
    /// doc) - its bind groups have to be made with this one.
    pub fn overlay_layout(&self, overlay: Overlay) -> &wgpu::BindGroupLayout {
        &self.overlay_layouts[&overlay]
    }

    fn fullscreen_key(settings: &ShaderSettings) -> PipelineKey {
        PipelineKey { cull: Cull::None, depth_write: false, blend: settings.blend() }
    }

    /// Says `problem` with the Fullscreen shader `name`, once.
    fn report_fullscreen(&mut self, name: &str, problem: &str) {
        let line = format!("\"{name}\": {problem}");
        if self.reported.insert(line.clone()) {
            eprintln!("Shader {line} - drawing the scene plain (engine/blit) instead");
        }
    }

    fn ensure_fullscreen_pipeline(&mut self, name: &str) -> Result<(), String> {
        let Some(Ok(shader)) = self.built.get(name) else { return Err("it isn't built".to_owned()) };
        let key = Self::fullscreen_key(&shader.settings);
        if shader.pipelines.contains_key(&key) {
            return Ok(());
        }
        let (Some(module), Some(Some(layout))) = (shader.module.clone(), shader.group0.as_ref().map(|group0| group0.pipeline_layout.clone())) else { return Err("it isn't built".to_owned()) };
        let blend = blend_state(key.blend);
        let format = self.color_format;
        let device = self.device.clone();
        let pipeline = caught(&device, || device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_main"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })).map_err(|error| format!("the GPU refused its pipeline: {error}"))?;
        if let Some(Ok(shader)) = self.built.get_mut(name) {
            shader.pipelines.insert(key, pipeline);
        }
        Ok(())
    }

    /// Every `.wgsl` under `folder` is a shader, named by its path in it
    /// without the extension ("ui/glitch").
    pub fn add_folder(&mut self, folder: impl Into<PathBuf>) {
        self.folders.push(folder.into());
    }

    /// The shader `name` from source - `settings` is its `.ron` text.
    pub fn add(&mut self, name: &str, wgsl: &str, settings: Option<&str>) {
        self.registered.insert(name.to_owned(), Source::Embedded { wgsl: wgsl.to_owned(), settings: settings.map(str::to_owned) });
        self.forget(name);
    }

    /// The shader `name` from the file `wgsl` (its `.ron` next to it).
    pub fn add_file(&mut self, name: &str, wgsl: impl Into<PathBuf>) {
        self.registered.insert(name.to_owned(), Source::File { wgsl: wgsl.into() });
        self.forget(name);
    }

    /// The built-in `builtin` ("engine/standard") is drawn with the game's
    /// shader `with` instead.
    pub fn override_builtin(&mut self, builtin: &str, with: &str) {
        self.overrides.insert(builtin.to_owned(), with.to_owned());
        self.forget(builtin);
    }

    fn forget(&mut self, name: &str) {
        self.changes += 1;
        self.built.remove(name);
        self.reported.retain(|line| !line.starts_with(&format!("\"{name}\"")));
    }

    /// The material `key` (a node's own: its shader, and the values it sets
    /// - see `Material::key`) - kept while it's in use (see
    /// `retain_instances`); its params block is rewritten when they change.
    pub fn set_instance(&mut self, key: &str, shader: &str, params: &BTreeMap<String, ParamValue>) {
        match self.instances.get_mut(key) {
            Some(instance) if instance.shader == shader => {
                if instance.params != *params {
                    instance.params = params.clone();
                }
            }
            _ => {
                self.instances.insert(key.to_owned(), Instance { shader: shader.to_owned(), params: params.clone(), buffer: None, bind_groups: HashMap::new(), generation: 0 });
            }
        }
    }

    /// Drops the material instances not in `keys` - the ones nothing uses
    /// any more.
    pub fn retain_instances(&mut self, keys: &HashSet<String>) {
        self.instances.retain(|key, _| keys.contains(key));
    }

    /// The shader the material `key` is drawn with (a key with no instance
    /// is a shader's name), after the game's overrides.
    fn shader_of(&self, key: &str) -> String {
        self.shader_name(key).to_owned()
    }

    /// `shader_of`, borrowed - for the lookups done for every mesh, every
    /// frame, which only read.
    fn shader_name<'a>(&'a self, key: &'a str) -> &'a str {
        let shader = self.instances.get(key).map_or(key, |instance| instance.shader.as_str());
        self.overrides.get(shader).map_or(shader, String::as_str)
    }

    /// Where the material `key` (see `set_instance` - or just a shader's
    /// name) is drawn, for a frame plan whose scene passes are `passes` -
    /// building its shader (and every pipeline it needs) if it isn't yet,
    /// and writing its params. Anything that keeps it from drawing makes it
    /// `Placement::Error`, said once in the console.
    pub fn placement(&mut self, key: &str, passes: &HashSet<String>) -> Placement {
        let placement = self.place_shader(&self.shader_of(key), passes);
        if placement != Placement::Error {
            self.write_params(key);
        }
        placement
    }

    fn place_shader(&mut self, name: &str, passes: &HashSet<String>) -> Placement {
        let name = name.to_owned();
        let problem = match self.ensure_built(&name) {
            Err(problem) => Some(problem),
            Ok(settings) => match (settings.kind, &settings.pass) {
                (Kind::Ui | Kind::Text | Kind::Lines, _) => Some(format!("it's a {:?} shader, used on a 3D model", settings.kind)),
                (Kind::Fullscreen, _) => Some("it's a Fullscreen shader, used on a 3D model".to_owned()),
                (_, Some(pass)) if !passes.contains(pass) => Some(format!("it draws in pass \"{pass}\" - this scene's frame plan has no such pass")),
                _ => None,
            },
        };
        if let Some(problem) = problem {
            self.report(&name, &problem);
            // If even the error material can't be built, it just isn't drawn.
            let _ = self.ensure_pipelines(ERROR, &Placement::Error);
            return Placement::Error;
        }
        let placement = match &self.built[&name].as_ref().expect("built just above").settings.pass {
            None => Placement::ByMesh,
            Some(pass) => Placement::Pass(pass.clone()),
        };
        if let Err(problem) = self.ensure_pipelines(&name, &placement) {
            self.report(&name, &problem);
            // If even the error material can't be built, it just isn't drawn.
            let _ = self.ensure_pipelines(ERROR, &Placement::Error);
            return Placement::Error;
        }
        placement
    }

    /// The material `key`'s pipeline `pipeline`, once `placement` has built
    /// it - engine/error's for a material placed as an error.
    pub fn pipeline(&self, key: &str, placement: &Placement, pipeline: PipelineKey) -> Option<&wgpu::RenderPipeline> {
        let name = if *placement == Placement::Error { ERROR } else { self.shader_name(key) };
        self.built.get(name)?.as_ref().ok()?.pipelines.get(&pipeline)
    }

    /// The settings of the material `key`'s shader, once `placement` has
    /// built it.
    pub fn settings(&self, key: &str) -> Option<&ShaderSettings> {
        self.built.get(self.shader_name(key))?.as_ref().ok().map(|shader| &shader.settings)
    }

    /// Whether the material `key`'s shader reads the depth so far (`reads:
    /// [Depth]`) - what a frame plan's CopyDepth is for.
    pub fn reads_depth(&self, key: &str) -> bool {
        self.settings(key).is_some_and(|settings| settings.reads.contains(&Read::Depth))
    }

    /// Whether the material `key`'s shader has a bind group 0 of its own
    /// (params or textures) - drawn with `group0`, not the mesh's.
    pub fn has_group0(&self, key: &str) -> bool {
        matches!(self.built.get(self.shader_name(key)), Some(Ok(shader)) if shader.group0.is_some())
    }

    /// Makes the material `key`'s bind group 0 for the mesh texture `diffuse`
    /// (index `index` in the model `model`) if it isn't made yet - once
    /// `placement` has built it and written its params.
    pub fn prepare_group0(&mut self, key: &str, model: &str, index: usize, diffuse: &Texture) {
        // Made already, nearly every time - checked first, without building
        // anything to look it up with.
        if self.group0(key, model, index).is_some() {
            return;
        }
        let name = self.shader_of(key);
        let Some(Ok(shader)) = self.built.get(&name) else { return };
        let Some(group0) = &shader.group0 else { return };
        let Some(instance) = self.instances.get_mut(key) else { return };
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&diffuse.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&diffuse.sampler) },
        ];
        if group0.params.is_some() {
            let Some((buffer, _)) = &instance.buffer else { return };
            entries.push(wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() });
        }
        for (index, texture) in group0.textures.iter().enumerate() {
            let binding = 3 + 2 * index as u32;
            entries.push(wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(&texture.view) });
            entries.push(wgpu::BindGroupEntry { binding: binding + 1, resource: wgpu::BindingResource::Sampler(&texture.sampler) });
        }
        if group0.depth {
            let Some((view, sampler)) = &self.depth_copy else { return };
            entries.push(wgpu::BindGroupEntry { binding: DEPTH_BINDING, resource: wgpu::BindingResource::TextureView(view) });
            entries.push(wgpu::BindGroupEntry { binding: DEPTH_BINDING + 1, resource: wgpu::BindingResource::Sampler(sampler) });
        }
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(key), layout: &group0.layout, entries: &entries });
        instance.bind_groups.entry(model.to_owned()).or_default().insert(index, bind_group);
    }

    /// The material `key`'s bind group 0 for the mesh texture `index` of
    /// the model `model` (see `prepare_group0`).
    pub fn group0(&self, key: &str, model: &str, index: usize) -> Option<&wgpu::BindGroup> {
        self.instances.get(key)?.bind_groups.get(model)?.get(&index)
    }

    /// Writes the material `key`'s params block - the shader's `.ron`
    /// defaults, then its own values - if they changed (or the shader was
    /// built again since). A shader with no params block has nothing to
    /// write.
    fn write_params(&mut self, key: &str) {
        let name = self.shader_of(key);
        let Some(Ok(shader)) = self.built.get(&name) else { return };
        let generation = shader.generation;
        if !self.instances.contains_key(key) {
            self.instances.insert(key.to_owned(), Instance { shader: key.to_owned(), params: BTreeMap::new(), buffer: None, bind_groups: HashMap::new(), generation });
        }
        let instance = self.instances.get_mut(key).expect("made just above");
        if instance.generation != generation {
            instance.generation = generation;
            instance.buffer = None;
            instance.bind_groups.clear();
        }
        let Some(layout) = shader.group0.as_ref().and_then(|group0| group0.params.as_ref()) else { return };
        let (bytes, unknown) = pack_params(layout, &shader.settings.params, &instance.params);
        let mut problems = Vec::new();
        for field in unknown {
            problems.push(format!("\"{key}\" note: it sets \"{field}\", which its shader's Params doesn't have"));
        }
        match &instance.buffer {
            Some((_, written)) if *written == bytes => {}
            Some((buffer, _)) => {
                self.queue.write_buffer(buffer, 0, &bytes);
                instance.buffer = Some((buffer.clone(), bytes));
            }
            None => {
                use wgpu::util::DeviceExt;
                let buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(key),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                });
                instance.buffer = Some((buffer, bytes));
                instance.bind_groups.clear();
            }
        }
        for problem in problems {
            if self.reported.insert(problem.clone()) {
                eprintln!("Shader {problem}");
            }
        }
    }

    /// Builds again whatever shader's files changed since they were read -
    /// at most once every WATCH_INTERVAL. Call once a frame.
    pub fn watch(&mut self) {
        if self.last_watch.elapsed() < WATCH_INTERVAL {
            return;
        }
        self.last_watch = Instant::now();
        let changed: Vec<String> = self.built.iter()
            .filter(|(_, built)| {
                let watch = match built {
                    Ok(shader) => &shader.watch,
                    Err(failure) => &failure.watch,
                };
                watch.iter().any(|(path, when)| modified(path) != *when)
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in changed {
            println!("Shader \"{name}\" changed - building it again");
            self.forget(&name);
        }
    }

    /// The shader `name`'s files as they are now - the ones it's read from,
    /// or where it'd be looked for if it isn't found.
    fn watch_of(&self, name: &str) -> Watch {
        let paths = match self.source(name) {
            Some(Source::File { wgsl }) => vec![wgsl.with_extension("ron"), wgsl],
            Some(_) => Vec::new(),
            None => self.folders.iter().map(|folder| folder.join(format!("{name}.wgsl"))).collect(),
        };
        paths.into_iter().map(|path| { let when = modified(&path); (path, when) }).collect()
    }

    /// Says `problem` with the shader `name`, once.
    fn report(&mut self, name: &str, problem: &str) {
        let line = format!("\"{name}\": {problem}");
        if self.reported.insert(line.clone()) {
            eprintln!("Shader {line} - drawing it with the error material");
        }
    }

    /// Where the shader `name`'s code comes from.
    fn source(&self, name: &str) -> Option<Source> {
        match name {
            STANDARD => return Some(Source::Standard),
            ERROR => return Some(Source::Error),
            BLIT => return Some(Source::Blit),
            UI => return Some(Source::Overlay { kind: Kind::Ui, error: false }),
            SDF_TEXT => return Some(Source::Overlay { kind: Kind::Text, error: false }),
            THICK_LINES => return Some(Source::Overlay { kind: Kind::Lines, error: false }),
            _ => {}
        }
        for kind in [Kind::Ui, Kind::Text, Kind::Lines] {
            if name == kind.error_shader() {
                return Some(Source::Overlay { kind, error: true });
            }
        }
        if let Some(source) = self.registered.get(name) {
            return Some(source.clone());
        }
        if name.starts_with("engine/") {
            return None;
        }
        self.folders.iter()
            .map(|folder| folder.join(format!("{name}.wgsl")))
            .find(|path| path.is_file())
            .map(|wgsl| Source::File { wgsl })
    }

    /// Builds the shader `name` if it isn't yet - its settings, or why it
    /// can't be.
    fn ensure_built(&mut self, name: &str) -> Result<&ShaderSettings, String> {
        if !self.built.contains_key(name) {
            let built = self.build(name).map_err(|problem| Failure { problem, watch: self.watch_of(name) });
            self.built.insert(name.to_owned(), built);
        }
        match &self.built[name] {
            Ok(shader) => Ok(&shader.settings),
            Err(failure) => Err(failure.problem.clone()),
        }
    }

    fn build(&mut self, name: &str) -> Result<Shader, String> {
        let Some(source) = self.source(name) else {
            let looked: Vec<String> = self.folders.iter().map(|folder| folder.join(format!("{name}.wgsl")).display().to_string()).collect();
            return Err(if name.starts_with("engine/") {
                "there's no such built-in shader".to_owned()
            } else if looked.is_empty() {
                "not found - no shader folder added (ShaderLibrary::add_folder)".to_owned()
            } else {
                format!("not found (looked for {})", looked.join(", "))
            });
        };
        let (wgsl, settings_text, origin, watch) = match source {
            Source::Standard => (String::new(), None, "engine/standard".to_owned(), Vec::new()),
            Source::Error => (ERROR_SOURCE.to_owned(), Some("(kind: Raw)".to_owned()), "engine/error".to_owned(), Vec::new()),
            Source::Blit => (BLIT_HOOKS.to_owned(), Some("(kind: Fullscreen, inputs: [\"scene\"])".to_owned()), "engine/blit".to_owned(), Vec::new()),
            Source::Overlay { kind, error } => (overlay_hooks(kind, error), Some(format!("(kind: {kind:?})")), name.to_owned(), Vec::new()),
            Source::Embedded { wgsl, settings } => (wgsl, settings, name.to_owned(), Vec::new()),
            Source::File { wgsl } => {
                let ron_path = wgsl.with_extension("ron");
                let watch = self.watch_of(name);
                let code = std::fs::read_to_string(&wgsl).map_err(|error| format!("couldn't read {}: {error}", wgsl.display()))?;
                let settings = if ron_path.is_file() {
                    Some(std::fs::read_to_string(&ron_path).map_err(|error| format!("couldn't read {}: {error}", ron_path.display()))?)
                } else {
                    None
                };
                (code, settings, wgsl.display().to_string(), watch)
            }
        };
        let settings: ShaderSettings = match &settings_text {
            Some(text) => read_settings(text).map_err(|error| format!("its settings (.ron) aren't valid: {error}"))?,
            None => ShaderSettings::default(),
        };
        self.generation += 1;
        self.changes += 1;
        let generation = self.generation;

        // Its textures - read now, watched with its files.
        let mut watch = watch;
        let mut textures = Vec::new();
        for (texture, path) in &settings.textures {
            if !is_identifier(texture) {
                return Err(format!("texture \"{texture}\" needs a name WGSL can use (letters, digits, _)"));
            }
            watch.push((PathBuf::from(path), modified(Path::new(path))));
            let bytes = std::fs::read(path).map_err(|error| format!("couldn't read its texture \"{texture}\" ({path}): {error}"))?;
            let loaded = Texture::from_bytes(&bytes, &self.device, &self.queue, texture).map_err(|error| format!("its texture \"{texture}\" ({path}) isn't an image it can read: {error}"))?;
            textures.push(loaded);
        }

        if !settings.kind.overlays().is_empty() {
            return self.build_overlay(name, &wgsl, &origin, settings, textures, watch, generation);
        }

        // The whole program, and where the shader's own lines start in it.
        let (program, first_line) = match settings.kind {
            Kind::Surface => assemble_surface_with(&wgsl, &surface_declarations(&wgsl, &settings)),
            Kind::Fullscreen => {
                if !code_of(&wgsl).contains("fn fragment(") {
                    return Err("a Fullscreen shader needs a `fn fragment(f: ScreenIn) -> vec4<f32>` hook".to_owned());
                }
                for input in &settings.inputs {
                    if !is_identifier(input) {
                        return Err(format!("input \"{input}\" needs a name WGSL can use"));
                    }
                }
                assemble_fullscreen(&wgsl, &fullscreen_declarations(&wgsl, &settings))
            }
            Kind::Raw => (wgsl.clone(), 1),
            Kind::Ui | Kind::Text | Kind::Lines => unreachable!("built above"),
        };
        let module = check(&program, first_line, wgsl.lines().count().max(1), &origin)?;
        let params = params_layout(&module)?;
        if params.is_none() && !settings.params.is_empty() {
            self.report_note(name, "its settings have params, but it has no `struct Params` (params at group 0 binding 2)");
        }
        if settings.kind == Kind::Raw {
            for (entry, stage) in [("vs_main", wgpu::naga::ShaderStage::Vertex), ("fs_main", wgpu::naga::ShaderStage::Fragment)] {
                if !module.entry_points.iter().any(|point| point.name == entry && point.stage == stage) {
                    return Err(format!("a Raw shader needs a `{entry}` entry point ({stage:?})"));
                }
            }
        }
        let device = self.device.clone();
        let module = caught(&device, || device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(name),
            source: wgpu::ShaderSource::Wgsl(program.into()),
        })).map_err(|error| format!("the GPU refused it: {error}"))?;
        let depth = settings.reads.contains(&Read::Depth);
        let fullscreen = settings.kind == Kind::Fullscreen;
        let inputs = if fullscreen { settings.inputs.clone() } else { Vec::new() };
        let group0 = (fullscreen || params.is_some() || !textures.is_empty() || depth).then(|| self.make_group0(name, params, textures, depth, settings.kind, inputs));
        Ok(Shader { settings, module: Some(module), pipelines: HashMap::new(), overlays: HashMap::new(), watch, group0, generation })
    }

    /// Builds a 2D shader - its hook in each of its kind's templates, and a
    /// pipeline for each (see the module doc).
    #[allow(clippy::too_many_arguments)]
    fn build_overlay(&mut self, name: &str, wgsl: &str, origin: &str, settings: ShaderSettings, textures: Vec<Texture>, watch: Watch, generation: u64) -> Result<Shader, String> {
        let kind = settings.kind;
        if !code_of(wgsl).contains("fn fragment(") {
            return Err(format!("a {kind:?} shader needs a `fn fragment(f: {}) -> vec4<f32>` hook", kind.input()));
        }
        if !settings.inputs.is_empty() {
            return Err("only a Fullscreen shader reads `inputs`".to_owned());
        }
        if !settings.reads.is_empty() {
            return Err(format!("a {kind:?} shader can't read {:?}", settings.reads));
        }
        let declarations = surface_declarations(wgsl, &settings);
        let mut programs = Vec::new();
        let mut params = None;
        for &overlay in kind.overlays() {
            let (program, first_line) = assemble_overlay(overlay, wgsl, &declarations);
            let module = check(&program, first_line, wgsl.lines().count().max(1), origin)?;
            params = params_layout(&module)?;
            programs.push((overlay, program));
        }
        if params.is_none() && !settings.params.is_empty() {
            self.report_note(name, "its settings have params, but it has no `struct Params` (params at group 0 binding 2)");
        }
        let group0 = self.make_group0(name, params, textures, false, kind, Vec::new());
        let blend = settings.blend();
        let mut overlays = HashMap::new();
        for (overlay, program) in programs {
            let device = self.device.clone();
            let module = caught(&device, || device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(program.into()),
            })).map_err(|error| format!("the GPU refused it: {error}"))?;
            let pipeline = caught(&device, || self.make_overlay_pipeline(name, &module, &group0.layout, overlay, blend)).map_err(|error| format!("the GPU refused its pipeline: {error}"))?;
            overlays.insert(overlay, pipeline);
        }
        Ok(Shader { settings, module: None, pipelines: HashMap::new(), overlays, watch, group0: Some(group0), generation })
    }

    fn make_overlay_pipeline(&self, name: &str, module: &wgpu::ShaderModule, group0: &wgpu::BindGroupLayout, overlay: Overlay, blend: Blend) -> wgpu::RenderPipeline {
        let layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(name),
            bind_group_layouts: &[group0, &self.groups[1], &self.overlay_layouts[&overlay], &self.groups[3]],
            push_constant_ranges: &[],
        });
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&layout),
            vertex: wgpu::VertexState { module, entry_point: Some("vs_main"), buffers: &[overlay.vertex_layout()], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: self.color_format, blend: blend_state(blend), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: overlay.cull(),
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        })
    }

    /// The bind group 0 layout of a shader with params, textures or the
    /// depth: the mesh's texture (0, 1), its params (2), its textures (3 on),
    /// the depth so far (DEPTH_BINDING, +1).
    fn make_group0(&self, name: &str, params: Option<ParamsLayout>, textures: Vec<Texture>, depth: bool, kind: Kind, inputs: Vec<String>) -> Group0 {
        let fullscreen = kind == Kind::Fullscreen;
        let mesh = matches!(kind, Kind::Surface | Kind::Raw);
        let visibility = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry { binding, visibility, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None };
        // Only a model's has the mesh's texture.
        let mut entries = if mesh { vec![texture_entry(0), sampler_entry(1)] } else { Vec::new() };
        if params.is_some() {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            });
        }
        for index in 0..(textures.len() + inputs.len()) as u32 {
            entries.push(texture_entry(3 + 2 * index));
            entries.push(sampler_entry(4 + 2 * index));
        }
        if depth {
            // Read as plain floats with no filtering - depth isn't blended.
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: DEPTH_BINDING,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: false } },
                count: None,
            });
            entries.push(wgpu::BindGroupLayoutEntry { binding: DEPTH_BINDING + 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering), count: None });
        }
        let layout = self.device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(name), entries: &entries });
        // A Fullscreen shader's group 1 is the light (for the time); a
        // model's are the camera, the mesh's transform, the light.
        // A 2D shader's are made with each pipeline (see make_overlay_pipeline).
        let pipeline_layout = if fullscreen {
            Some(self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(name), bind_group_layouts: &[&layout, &self.groups[3]], push_constant_ranges: &[] }))
        } else if mesh {
            Some(self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some(name), bind_group_layouts: &[&layout, &self.groups[1], &self.groups[2], &self.groups[3]], push_constant_ranges: &[] }))
        } else {
            None
        };
        Group0 { layout, pipeline_layout, params, textures, depth, inputs }
    }

    /// Says a note about the shader `name`, once (not an error).
    fn report_note(&mut self, name: &str, note: &str) {
        let line = format!("\"{name}\" note: {note}");
        if self.reported.insert(line.clone()) {
            eprintln!("Shader {line}");
        }
    }

    /// Builds every pipeline the shader `name` needs placed like
    /// `placement` - failing, it's an error from then on.
    fn ensure_pipelines(&mut self, name: &str, placement: &Placement) -> Result<(), String> {
        if name == ERROR && !self.built.contains_key(ERROR) {
            self.ensure_built(ERROR).map(|_| ())?;
        }
        let Some(Ok(shader)) = self.built.get(name) else {
            return Err("it isn't built".to_owned());
        };
        let missing: Vec<PipelineKey> = placement.keys(&shader.settings).into_iter().filter(|key| !shader.pipelines.contains_key(key)).collect();
        if missing.is_empty() {
            return Ok(());
        }
        let Some(module) = shader.module.clone() else { return Ok(()) };
        let layout = shader.group0.as_ref().and_then(|group0| group0.pipeline_layout.as_ref()).unwrap_or(&self.layout).clone();
        let mut made = Vec::new();
        for key in missing {
            let pipeline = caught(&self.device, || self.make_pipeline(name, &module, &layout, key)).map_err(|error| format!("the GPU refused its pipeline: {error}"));
            match pipeline {
                Ok(pipeline) => made.push((key, pipeline)),
                Err(problem) => {
                    let watch = self.built.get(name).and_then(|shader| shader.as_ref().ok()).map(|shader| shader.watch.clone()).unwrap_or_default();
                    self.built.insert(name.to_owned(), Err(Failure { problem: problem.clone(), watch }));
                    return Err(problem);
                }
            }
        }
        if let Some(Ok(shader)) = self.built.get_mut(name) {
            shader.pipelines.extend(made);
        }
        Ok(())
    }

    fn make_pipeline(&self, name: &str, module: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, key: PipelineKey) -> wgpu::RenderPipeline {
        let blend = blend_state(key.blend);
        let cull_mode = match key.cull {
            Cull::Back => Some(wgpu::Face::Back),
            Cull::Front => Some(wgpu::Face::Front),
            Cull::None => None,
        };
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                buffers: &[model::ModelVertex::desc(), InstanceRaw::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: self.color_format, blend, write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Texture::DEPTH_FORMAT,
                // See-through geometry still tests depth (what's solid in
                // front hides it) but doesn't write it, so its faces don't
                // hide each other.
                depth_write_enabled: key.depth_write,
                // Reversed-Z: near = 1.0, far = 0.0, so "closer" means "greater".
                depth_compare: wgpu::CompareFunction::Greater,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        })
    }
}

/// A Surface shader's hooks pasted into the template (with the defaults for
/// any it leaves out) - and the line its own code starts on.
#[cfg(test)]
fn assemble_surface(hooks: &str) -> (String, usize) {
    assemble_surface_with(hooks, "")
}

/// The shader's code without its `//` comments - for telling what it has.
fn code_of(source: &str) -> String {
    source.lines().map(|line| line.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n")
}

/// What the engine declares for a Surface shader's own values (see the
/// module doc): `params` if it has a `struct Params`, and each of its
/// settings' textures with its sampler.
fn surface_declarations(hooks: &str, settings: &ShaderSettings) -> String {
    let mut declarations = String::new();
    if code_of(hooks).contains("struct Params") {
        declarations.push_str("@group(0) @binding(2)\nvar<uniform> params: Params;\n");
    }
    for (index, texture) in settings.textures.keys().enumerate() {
        let binding = 3 + 2 * index;
        declarations.push_str(&format!("@group(0) @binding({binding})\nvar {texture}: texture_2d<f32>;\n@group(0) @binding({})\nvar {texture}_sampler: sampler;\n", binding + 1));
    }
    declarations
}

/// `assemble_surface`, with `declarations` after the shader's own code (so
/// its line numbers stay its own).
fn assemble_surface_with(hooks: &str, declarations: &str) -> (String, usize) {
    let code = code_of(hooks);
    let mut user = hooks.to_owned();
    if !user.ends_with('\n') {
        user.push('\n');
    }
    if !code.contains("fn vertex(") {
        user.push_str(DEFAULT_VERTEX);
    }
    if !code.contains("fn fragment(") {
        user.push_str(DEFAULT_FRAGMENT);
    }
    user.push_str(declarations);
    let marker = TEMPLATE.find(HOOKS_MARKER).expect("the template has its hooks marker");
    let first_line = TEMPLATE[..marker].lines().count() + 1;
    let program = format!("{}{}{}", &TEMPLATE[..marker], user, &TEMPLATE[marker + HOOKS_MARKER.len()..]);
    (program, first_line)
}

/// `program` checked with naga - its module, or what's wrong and where: a
/// line in the shader's own file (`origin`, whose `user_lines` lines start
/// at `first_line` of the program) when that's where it is.
fn check(program: &str, first_line: usize, user_lines: usize, origin: &str) -> Result<wgpu::naga::Module, String> {
    let at = |line: u32| -> String {
        let line = line as usize;
        if line >= first_line && line < first_line + user_lines {
            format!("{origin}:{}", line - first_line + 1)
        } else {
            format!("the engine's template, line {line}")
        }
    };
    let module = wgpu::naga::front::wgsl::parse_str(program).map_err(|error| {
        let place = error.location(program).map(|location| at(location.line_number)).unwrap_or_else(|| origin.to_owned());
        format!("doesn't compile - {place}: {}", error.message())
    })?;
    wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all())
        .validate(&module)
        .map_err(|error| {
            let place = error.location(program).map(|location| at(location.line_number)).unwrap_or_else(|| origin.to_owned());
            format!("isn't valid - {place}: {}", error.as_inner())
        })?;
    Ok(module)
}

/// A shader's `.ron` - `pass: "x"` for an optional field, like the game's
/// other data files.
fn read_settings(text: &str) -> Result<ShaderSettings, ron::error::SpannedError> {
    ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME).from_str(text)
}

/// What the engine declares for a Fullscreen shader: `params` if it has a
/// `struct Params`, its textures, then its inputs - each with its sampler.
fn fullscreen_declarations(hooks: &str, settings: &ShaderSettings) -> String {
    let mut declarations = surface_declarations(hooks, settings);
    let first = settings.textures.len();
    for (index, input) in settings.inputs.iter().enumerate() {
        let binding = 3 + 2 * (first + index);
        declarations.push_str(&format!("@group(0) @binding({binding})\nvar {input}: texture_2d<f32>;\n@group(0) @binding({})\nvar {input}_sampler: sampler;\n", binding + 1));
    }
    declarations
}

/// A Fullscreen shader's hook pasted into its template, `declarations`
/// after it - and the line its own code starts on.
fn assemble_fullscreen(hooks: &str, declarations: &str) -> (String, usize) {
    let mut user = hooks.to_owned();
    if !user.ends_with('\n') {
        user.push('\n');
    }
    user.push_str(declarations);
    let marker = FULLSCREEN_TEMPLATE.find(HOOKS_MARKER).expect("the fullscreen template has its hooks marker");
    let first_line = FULLSCREEN_TEMPLATE[..marker].lines().count() + 1;
    (format!("{}{}{}", &FULLSCREEN_TEMPLATE[..marker], user, &FULLSCREEN_TEMPLATE[marker + HOOKS_MARKER.len()..]), first_line)
}

/// The hook of the engine's own 2D shader of `kind` - the color as the
/// engine works it out, or (`error`) that shape in flashing magenta, like
/// engine/error.
fn overlay_hooks(kind: Kind, error: bool) -> String {
    let input = kind.input();
    if error {
        format!("fn fragment(f: {input}) -> vec4<f32> {{\n    let brightness = select(0.5, 1.0, fract(engine.time * 2.0) < 0.5);\n    return vec4<f32>(vec3<f32>(1.0, 0.0, 1.0) * brightness, f.color.a);\n}}\n")
    } else {
        format!("fn fragment(f: {input}) -> vec4<f32> {{\n    return f.color;\n}}\n")
    }
}

/// A 2D shader's hook pasted into `overlay`'s template, `declarations` after
/// it - and the line its own code starts on.
fn assemble_overlay(overlay: Overlay, hooks: &str, declarations: &str) -> (String, usize) {
    let template = overlay.template();
    let mut user = hooks.to_owned();
    if !user.ends_with('\n') {
        user.push('\n');
    }
    user.push_str(declarations);
    let marker = template.find(HOOKS_MARKER).expect("every 2D template has its hooks marker");
    let first_line = template[..marker].lines().count() + 1;
    (format!("{}{}{}", &template[..marker], user, &template[marker + HOOKS_MARKER.len()..]), first_line)
}

/// The GPU blend state for `blend`.
fn blend_state(blend: Blend) -> Option<wgpu::BlendState> {
    match blend {
        Blend::Alpha => Some(wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
        }),
        Blend::Additive => Some(wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        }),
        Blend::None => None,
    }
}

/// Whether `name` can be a WGSL name - letters, digits and `_`, not
/// starting with a digit.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_alphabetic() || first == '_') && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The `Params` block of a checked shader - the uniform at group 0, binding
/// 2 - field by field. None if it has none; an error for a field it can't
/// fill (anything but numbers, vectors of them, and arrays of those).
fn params_layout(module: &wgpu::naga::Module) -> Result<Option<ParamsLayout>, String> {
    use wgpu::naga::{AddressSpace, ArraySize, TypeInner};
    let Some(variable) = module.global_variables.iter().map(|(_, variable)| variable).find(|variable| {
        variable.space == AddressSpace::Uniform && variable.binding.as_ref().is_some_and(|binding| binding.group == 0 && binding.binding == 2)
    }) else {
        return Ok(None);
    };
    let TypeInner::Struct { members, span } = &module.types[variable.ty].inner else {
        return Err("its params (group 0, binding 2) have to be a struct".to_owned());
    };
    let shape = |inner: &TypeInner| -> Option<(wgpu::naga::ScalarKind, u32)> {
        match inner {
            TypeInner::Scalar(scalar) => Some((scalar.kind, 1)),
            TypeInner::Vector { size, scalar } => Some((scalar.kind, *size as u32)),
            _ => None,
        }
    };
    let mut fields = Vec::new();
    for member in members {
        let name = member.name.clone().unwrap_or_default();
        let inner = &module.types[member.ty].inner;
        let (kind, components, array) = match inner {
            TypeInner::Array { base, size: ArraySize::Constant(count), stride } => {
                let (kind, components) = shape(&module.types[*base].inner).ok_or_else(|| format!("params field \"{name}\" is an array of something that isn't numbers or vectors"))?;
                (kind, components, Some((count.get(), *stride)))
            }
            _ => {
                let (kind, components) = shape(inner).ok_or_else(|| format!("params field \"{name}\" isn't a number, a vector or an array of them"))?;
                (kind, components, None)
            }
        };
        if kind == wgpu::naga::ScalarKind::Bool {
            return Err(format!("params field \"{name}\" is a bool - use a u32 or an f32"));
        }
        fields.push(ParamField { name, offset: member.offset, kind, components, array });
    }
    // Uniform blocks come in 16-byte steps.
    Ok(Some(ParamsLayout { size: (*span as u64).div_ceil(16).max(1) * 16, fields }))
}

/// A material's params block as bytes - each field from `own` (the
/// material's values), else `defaults` (the shader's `.ron`), else 0 - and
/// the names `own` sets that aren't fields.
fn pack_params(layout: &ParamsLayout, defaults: &BTreeMap<String, ParamValue>, own: &BTreeMap<String, ParamValue>) -> (Vec<u8>, Vec<String>) {
    let mut bytes = vec![0u8; layout.size as usize];
    for field in &layout.fields {
        let Some(values) = own.get(&field.name).or_else(|| defaults.get(&field.name)) else { continue };
        let (count, stride) = field.array.unwrap_or((1, 0));
        for element in 0..count {
            for component in 0..field.components {
                let Some(value) = values.0.get((element * field.components + component) as usize) else { break };
                let at = (field.offset + element * stride + component * 4) as usize;
                let word = match field.kind {
                    wgpu::naga::ScalarKind::Sint => (*value as i32).to_le_bytes(),
                    wgpu::naga::ScalarKind::Uint => (value.max(0.0) as u32).to_le_bytes(),
                    _ => value.to_le_bytes(),
                };
                if let Some(slot) = bytes.get_mut(at..at + 4) {
                    slot.copy_from_slice(&word);
                }
            }
        }
    }
    let unknown = own.keys().filter(|name| !layout.fields.iter().any(|field| &field.name == *name)).cloned().collect();
    (bytes, unknown)
}

/// When the file at `path` was last changed - None if it isn't there.
fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Runs `make` inside a GPU validation error scope - what it made, or the
/// GPU's error. On the desktop backends the scope's result is there at
/// once, so it's polled once instead of awaited.
fn caught<T>(device: &wgpu::Device, make: impl FnOnce() -> T) -> Result<T, String> {
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = make();
    let mut scope = std::pin::pin!(device.pop_error_scope());
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match scope.as_mut().poll(&mut context) {
        std::task::Poll::Ready(Some(error)) => Err(error.to_string()),
        _ => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(program: &str) -> Result<(), String> {
        let module = wgpu::naga::front::wgsl::parse_str(program).map_err(|error| error.emit_to_string(program))?;
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all())
            .validate(&module)
            .map(|_| ())
            .map_err(|error| format!("{error:?}"))
    }

    #[test]
    fn engine_standard_is_valid() {
        validate(&assemble_surface("").0).unwrap();
    }

    #[test]
    fn engine_blit_is_valid() {
        let settings = read_settings("(kind: Fullscreen, inputs: [\"scene\"])").unwrap();
        let (program, first_line) = assemble_fullscreen(BLIT_HOOKS, &fullscreen_declarations(BLIT_HOOKS, &settings));
        check(&program, first_line, BLIT_HOOKS.lines().count(), "engine/blit").unwrap();
    }

    #[test]
    fn the_engine_2d_shaders_are_valid() {
        for kind in [Kind::Ui, Kind::Text, Kind::Lines] {
            for error in [false, true] {
                let hooks = overlay_hooks(kind, error);
                for &overlay in kind.overlays() {
                    let (program, first_line) = assemble_overlay(overlay, &hooks, "");
                    check(&program, first_line, hooks.lines().count(), &format!("{overlay:?}")).unwrap_or_else(|problem| panic!("{problem}"));
                }
            }
        }
    }

    #[test]
    fn a_2d_hook_gets_params_and_engine_values() {
        let hooks = "struct Params {\n    tint: vec4<f32>,\n};\nfn fragment(f: UiIn) -> vec4<f32> {\n    return f.color * params.tint * (0.5 + 0.5 * sin(engine.time + f.uv.x));\n}\n";
        let settings = read_settings("(kind: Ui, params: { \"tint\": (1, 1, 1, 1) })").unwrap();
        assert_eq!(settings.blend(), Blend::Alpha, "2D shaders blend unless told otherwise");
        for &overlay in Kind::Ui.overlays() {
            let (program, first_line) = assemble_overlay(overlay, hooks, &surface_declarations(hooks, &settings));
            let module = check(&program, first_line, hooks.lines().count(), "test").unwrap();
            assert!(params_layout(&module).unwrap().is_some());
        }
    }

    #[test]
    fn the_toast_icons_are_valid() {
        let settings = read_settings("(kind: Ui)").unwrap();
        for hooks in [include_str!("../shaders/ui_icon_alert.wgsl"), include_str!("../shaders/ui_icon_error.wgsl"), include_str!("../shaders/ui_icon_chevron.wgsl")] {
            for &overlay in Kind::Ui.overlays() {
                let (program, first_line) = assemble_overlay(overlay, hooks, &surface_declarations(hooks, &settings));
                check(&program, first_line, hooks.lines().count(), "icon").unwrap();
            }
        }
    }

    #[test]
    fn the_error_material_is_valid() {
        validate(ERROR_SOURCE).unwrap();
    }

    #[test]
    fn a_fragment_hook_alone_gets_the_default_vertex() {
        let hooks = "fn fragment(f: FragmentIn) -> vec4<f32> {\n    return vec4<f32>(f.lit_color.rgb * 0.5, 1.0);\n}\n";
        let (program, first_line) = assemble_surface(hooks);
        validate(&program).unwrap();
        assert_eq!(program.lines().nth(first_line - 1), Some("fn fragment(f: FragmentIn) -> vec4<f32> {"), "the hooks start at the line it says");
    }

    #[test]
    fn hooks_can_read_the_engine_values() {
        let hooks = "fn vertex(v: VertexIn) -> VertexIn {\n    var out = v;\n    out.position.y += sin(engine.time);\n    return out;\n}\nfn fragment(f: FragmentIn) -> vec4<f32> {\n    return vec4<f32>(engine.sun_color * f.base_color.rgb, 1.0);\n}\n";
        validate(&assemble_surface(hooks).0).unwrap();
    }

    #[test]
    fn params_are_laid_out_and_packed_by_field() {
        let hooks = "struct Params {\n    tint: vec4<f32>,\n    speed: f32,\n    count: u32,\n    wake: array<vec4<f32>, 3>,\n};\nfn fragment(f: FragmentIn) -> vec4<f32> {\n    return params.tint * params.speed + params.wake[1];\n}\n";
        let settings = read_settings("(params: { \"tint\": (0.2, 0.8, 1.0, 0.7), \"speed\": 4.0 })").unwrap();
        let (program, first_line) = assemble_surface_with(hooks, &surface_declarations(hooks, &settings));
        let module = check(&program, first_line, hooks.lines().count(), "test").unwrap();
        let layout = params_layout(&module).unwrap().expect("a params block");
        let names: Vec<&str> = layout.fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(names, ["tint", "speed", "count", "wake"]);
        assert_eq!(layout.fields[3].array, Some((3, 16)));
        // The material's own speed and wake; the .ron's tint; count left 0.
        let own: BTreeMap<String, ParamValue> = [("speed".to_owned(), ParamValue::from(2.5)), ("wake".to_owned(), ParamValue::from(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0])), ("tnit".to_owned(), ParamValue::from(1.0))].into();
        let (bytes, unknown) = pack_params(&layout, &settings.params, &own);
        let float_at = |offset: u32| f32::from_le_bytes(bytes[offset as usize..offset as usize + 4].try_into().unwrap());
        assert_eq!(float_at(layout.fields[0].offset + 4), 0.8, "tint from the .ron");
        assert_eq!(float_at(layout.fields[1].offset), 2.5, "speed from the material");
        assert_eq!(float_at(layout.fields[3].offset + 16), 5.0, "wake's second element");
        assert_eq!(unknown, ["tnit"], "a misspelled one is said");
        assert_eq!(bytes.len() % 16, 0);
    }

    #[test]
    fn textures_are_declared_for_the_hooks() {
        let hooks = "fn fragment(f: FragmentIn) -> vec4<f32> {\n    return textureSample(noise, noise_sampler, f.uv);\n}\n";
        let settings = read_settings("(textures: { \"noise\": \"assets/textures/noise.png\" })").unwrap();
        let (program, first_line) = assemble_surface_with(hooks, &surface_declarations(hooks, &settings));
        check(&program, first_line, hooks.lines().count(), "test").unwrap();
    }

    #[test]
    fn param_values_read_from_ron() {
        let values: BTreeMap<String, ParamValue> = ron::from_str("{ \"a\": 1.5, \"b\": (1, 2, 3), \"c\": [(1, 0), (0, 1)], \"d\": true }").unwrap();
        assert_eq!(values["a"].0, [1.5]);
        assert_eq!(values["b"].0, [1.0, 2.0, 3.0]);
        assert_eq!(values["c"].0, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(values["d"].0, [1.0]);
    }

    #[test]
    fn every_game_shader_builds() {
        fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(folder) else { return };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, found);
                } else if path.extension().is_some_and(|extension| extension == "wgsl") {
                    found.push(path);
                }
            }
        }
        let mut shaders = Vec::new();
        // The examples' shaders - the game code there.
        walk(Path::new("examples"), &mut shaders);
        for wgsl in shaders {
            let code = std::fs::read_to_string(&wgsl).unwrap();
            let ron_path = wgsl.with_extension("ron");
            let settings = if ron_path.is_file() { read_settings(&std::fs::read_to_string(&ron_path).unwrap()).unwrap_or_else(|error| panic!("{}: {error}", ron_path.display())) } else { ShaderSettings::default() };
            let origin = wgsl.display().to_string();
            match settings.kind {
                Kind::Surface => {
                    let (program, first_line) = assemble_surface_with(&code, &surface_declarations(&code, &settings));
                    check(&program, first_line, code.lines().count().max(1), &origin).unwrap_or_else(|problem| panic!("{problem}"));
                }
                Kind::Raw => { check(&code, 1, code.lines().count().max(1), &origin).unwrap_or_else(|problem| panic!("{problem}")); }
                Kind::Fullscreen => {
                    let (program, first_line) = assemble_fullscreen(&code, &fullscreen_declarations(&code, &settings));
                    check(&program, first_line, code.lines().count().max(1), &origin).unwrap_or_else(|problem| panic!("{problem}"));
                }
                Kind::Ui | Kind::Text | Kind::Lines => {
                    for &overlay in settings.kind.overlays() {
                        let (program, first_line) = assemble_overlay(overlay, &code, &surface_declarations(&code, &settings));
                        check(&program, first_line, code.lines().count().max(1), &origin).unwrap_or_else(|problem| panic!("{problem}"));
                    }
                }
            }
        }
    }

    #[test]
    fn a_mistake_is_found_on_its_own_line() {
        // Line 2 of the shader's own file calls something that doesn't exist.
        let hooks = "fn fragment(f: FragmentIn) -> vec4<f32> {
    return no_such_function(f.lit_color);
}
";
        let (program, first_line) = assemble_surface(hooks);
        let problem = check(&program, first_line, hooks.lines().count(), "assets/shaders/broken.wgsl").unwrap_err();
        assert!(problem.contains("assets/shaders/broken.wgsl:2"), "{problem}");
    }

    #[test]
    fn settings_read_from_ron_and_refuse_typos() {
        let settings = read_settings("(kind: Surface, pass: \"transparent\", blend: Alpha, cull: None, depth_write: false, params: { \"tint\": (0.2, 0.8, 1.0, 0.7) })").unwrap();
        assert_eq!(settings.pass.as_deref(), Some("transparent"));
        assert_eq!((settings.blend(), settings.cull, settings.depth_write), (Blend::Alpha, Cull::None, false));
        assert!(read_settings("(kidn: Surface)").is_err(), "a misspelled field");
        let defaults = read_settings("()").unwrap();
        assert_eq!((defaults.kind, defaults.pass, defaults.cull, defaults.depth_write), (Kind::Surface, None, Cull::Back, true));
    }
}
