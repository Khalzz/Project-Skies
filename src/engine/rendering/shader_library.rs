//! Named shaders - what a node's `Material` (see scene_manager::properties)
//! draws its model with.
//!
//! - Built in, under `engine/`: "engine/standard" (what every model is drawn
//!   with unless it says otherwise), "engine/water" (the ocean - drawn by the
//!   water renderer, see render_pass's water pass) and "engine/error" (see
//!   below).
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
//! A shader is read and built the first time a material asks for it -
//! checked with naga first, so a mistake comes back with the line in the
//! shader's own file, then built inside a GPU error scope. One that's
//! missing or doesn't work (or is the wrong kind, or wants a pass the frame
//! plan doesn't have) draws with "engine/error" - flashing magenta - and
//! says why in the console, once. Its files are watched: saved, it's built
//! again.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::Deserialize;

use crate::engine::rendering::instance_management::InstanceRaw;
use crate::engine::rendering::models::model::{self, Vertex};
use crate::engine::rendering::models::textures::Texture;

pub const STANDARD: &str = "engine/standard";
pub const WATER: &str = "engine/water";
pub const ERROR: &str = "engine/error";

const TEMPLATE: &str = include_str!("../shaders/standard_template.wgsl");
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
    /// Hooks for UI elements.
    Ui,
    /// A whole shader, taking the engine's standard inputs.
    Raw,
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
    pub blend: Blend,
    pub cull: Cull,
    pub depth_write: bool,
    /// Defaults for the shader's own parameters, by name.
    pub params: HashMap<String, ron::Value>,
    /// Textures it reads, by name.
    pub textures: HashMap<String, String>,
    pub reads: Vec<Read>,
    /// Fixed-size arrays it reads, by name.
    pub arrays: HashMap<String, ron::Value>,
}

impl Default for ShaderSettings {
    fn default() -> Self {
        Self {
            kind: Kind::Surface,
            pass: None,
            blend: Blend::None,
            cull: Cull::Back,
            depth_write: true,
            params: HashMap::new(),
            textures: HashMap::new(),
            reads: Vec::new(),
            arrays: HashMap::new(),
        }
    }
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
    /// The ocean - the water pass draws it its own way.
    Water,
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
                PipelineKey { cull: settings.cull, depth_write: settings.depth_write, blend: settings.blend },
                PipelineKey { cull: Cull::None, depth_write: settings.depth_write, blend: settings.blend },
            ],
            Placement::Error => vec![
                PipelineKey { cull: Cull::Back, depth_write: true, blend: Blend::None },
                PipelineKey { cull: Cull::None, depth_write: true, blend: Blend::None },
            ],
            Placement::Water => Vec::new(),
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
    /// engine/water - no pipeline here (see `Placement::Water`).
    Water,
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
    /// Its files - built again when one changes.
    watch: Watch,
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
    color_format: wgpu::TextureFormat,
    layout: wgpu::PipelineLayout,
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
    /// For models drawn into `color_format` with `layout` - the model
    /// pipelines' bind groups (texture, camera, transform, light).
    pub fn new(device: wgpu::Device, color_format: wgpu::TextureFormat, layout: wgpu::PipelineLayout) -> Self {
        Self {
            device,
            color_format,
            layout,
            folders: Vec::new(),
            registered: HashMap::new(),
            overrides: HashMap::new(),
            built: HashMap::new(),
            reported: HashSet::new(),
            last_watch: Instant::now(),
        }
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
        self.built.remove(name);
        self.reported.retain(|line| !line.starts_with(&format!("\"{name}\"")));
    }

    /// Where a material using the shader `name` is drawn, for a frame plan
    /// whose scene passes are `passes` - building it (and every pipeline it
    /// needs) if it isn't yet. Anything that keeps it from drawing makes it
    /// `Placement::Error`, said once in the console.
    pub fn placement(&mut self, name: &str, passes: &HashSet<String>) -> Placement {
        let name = self.overrides.get(name).cloned().unwrap_or_else(|| name.to_owned());
        if name == WATER {
            return Placement::Water;
        }
        let problem = match self.ensure_built(&name) {
            Err(problem) => Some(problem),
            Ok(settings) => match (settings.kind, &settings.pass) {
                (Kind::Ui, _) => Some("it's a Ui shader, used on a 3D model".to_owned()),
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

    /// The shader `name`'s pipeline `key`, once `placement` has built it -
    /// engine/error's for a material placed as an error.
    pub fn pipeline(&self, name: &str, placement: &Placement, key: PipelineKey) -> Option<&wgpu::RenderPipeline> {
        let name = if *placement == Placement::Error { ERROR } else { self.overrides.get(name).map_or(name, String::as_str) };
        self.built.get(name)?.as_ref().ok()?.pipelines.get(&key)
    }

    /// A shader's settings, once `placement` has built it.
    pub fn settings(&self, name: &str) -> Option<&ShaderSettings> {
        let name = self.overrides.get(name).map_or(name, String::as_str);
        self.built.get(name)?.as_ref().ok().map(|shader| &shader.settings)
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
            WATER => return Some(Source::Water),
            _ => {}
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
            Source::Water => return Ok(Shader { settings: ShaderSettings { kind: Kind::Raw, pass: Some("water".to_owned()), ..Default::default() }, module: None, pipelines: HashMap::new(), watch: Vec::new() }),
            Source::Standard => (String::new(), None, "engine/standard".to_owned(), Vec::new()),
            Source::Error => (ERROR_SOURCE.to_owned(), Some("(kind: Raw)".to_owned()), "engine/error".to_owned(), Vec::new()),
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
        if !settings.params.is_empty() || !settings.textures.is_empty() || !settings.arrays.is_empty() {
            self.report_note(name, "params, textures and arrays in its settings aren't applied yet - they come with material parameters");
        }
        if !settings.reads.is_empty() {
            self.report_note(name, "`reads` isn't available to game shaders yet");
        }

        // The whole program, and where the shader's own lines start in it.
        let (program, first_line) = match settings.kind {
            Kind::Surface => assemble_surface(&wgsl),
            Kind::Raw => (wgsl.clone(), 1),
            Kind::Ui => return Ok(Shader { settings, module: None, pipelines: HashMap::new(), watch }),
        };
        let module = check(&program, first_line, wgsl.lines().count().max(1), &origin)?;
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
        Ok(Shader { settings, module: Some(module), pipelines: HashMap::new(), watch })
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
        let mut made = Vec::new();
        for key in missing {
            let pipeline = caught(&self.device, || self.make_pipeline(name, &module, key)).map_err(|error| format!("the GPU refused its pipeline: {error}"));
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

    fn make_pipeline(&self, name: &str, module: &wgpu::ShaderModule, key: PipelineKey) -> wgpu::RenderPipeline {
        let blend = match key.blend {
            Blend::Alpha => Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
            }),
            Blend::Additive => Some(wgpu::BlendState {
                color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
                alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            }),
            Blend::None => None,
        };
        let cull_mode = match key.cull {
            Cull::Back => Some(wgpu::Face::Back),
            Cull::Front => Some(wgpu::Face::Front),
            Cull::None => None,
        };
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(name),
            layout: Some(&self.layout),
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
fn assemble_surface(hooks: &str) -> (String, usize) {
    let code: String = hooks.lines().map(|line| line.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
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
        walk(Path::new("assets/shaders"), &mut shaders);
        for wgsl in shaders {
            let code = std::fs::read_to_string(&wgsl).unwrap();
            let ron_path = wgsl.with_extension("ron");
            let settings = if ron_path.is_file() { read_settings(&std::fs::read_to_string(&ron_path).unwrap()).unwrap_or_else(|error| panic!("{}: {error}", ron_path.display())) } else { ShaderSettings::default() };
            let origin = wgsl.display().to_string();
            match settings.kind {
                Kind::Surface => {
                    let (program, first_line) = assemble_surface(&code);
                    check(&program, first_line, code.lines().count().max(1), &origin).unwrap_or_else(|problem| panic!("{problem}"));
                }
                Kind::Raw => { check(&code, 1, code.lines().count().max(1), &origin).unwrap_or_else(|problem| panic!("{problem}")); }
                Kind::Ui => {}
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
        assert_eq!((settings.blend, settings.cull, settings.depth_write), (Blend::Alpha, Cull::None, false));
        assert!(read_settings("(kidn: Surface)").is_err(), "a misspelled field");
        let defaults = read_settings("()").unwrap();
        assert_eq!((defaults.kind, defaults.pass, defaults.cull, defaults.depth_write), (Kind::Surface, None, Cull::Back, true));
    }
}
