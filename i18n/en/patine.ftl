# Patine — English strings.
# Every key must also exist in i18n/fr/patine.ftl (checked by a test).

app-title = Patine

## Menus
menu-file = File
menu-edit = Edit
menu-view = View
menu-language = Language
menu-help = Help

## Actions
action-view-frame = Frame mesh
action-view-orthographic = Orthographic
action-view-wireframe = Wireframe
action-view-shadows = Shadows
action-file-new-project = New project…
action-file-open = Open…
action-file-save = Save
action-file-save-as = Save as…
action-app-quit = Quit
action-view-reset-layout = Reset layout
action-debug-toggle-profiler = Profiler
action-language-english = English
action-language-french = Français
action-help-about = About Patine
action-edit-undo = Undo
action-edit-redo = Redo

## Panels
panel-viewport-3d = 3D Viewport
panel-view-2d = 2D View
panel-texture-sets = Texture Sets
panel-layers = Layers
panel-properties = Properties
panel-library = Library
panel-display = Display Settings
panel-history = History
panel-console = Console
panel-empty = Nothing here yet.

## Console
console-clear = Clear
console-level = Minimum level
console-empty = No messages.
console-level-error = Errors
console-level-warn = Warnings
console-level-info = Information
console-level-debug = Debug
console-level-trace = Trace
console-profiler-on = Profiler on: connect puffin_viewer to 127.0.0.1:{ $port }

## About
about-description = Layer-based PBR texturing on 3D meshes.
about-version = Version { $version }
about-close = Close

## Status bar
status-gpu = GPU: { $name } ({ $backend })
status-project = Project: { $name }
status-hover = Triangle { $triangle } · polygon { $polygon } · { $object }

## New project
new-project-title = New project
new-project-mesh = Mesh
new-project-browse = Browse…
new-project-filter = 3D meshes
new-project-resolution = Resolution
new-project-normal-convention = Normal map
normal-convention-opengl = OpenGL (Y+)
normal-convention-directx = DirectX (Y−)
new-project-create = Create
new-project-cancel = Cancel
new-project-importing = Importing { $file }…

## Texture sets panel
texture-sets-empty = No project yet. Create one with File > New project (Ctrl+N).
texture-sets-mesh = { $name }: { $triangles } triangles, { $vertices } vertices
texture-sets-name = Texture set
texture-sets-resolution = Resolution
texture-sets-triangles = Triangles
texture-sets-rename-hint = Double-click to rename

## Mesh import errors
error-mesh-io = Cannot read { $path }: { $message }
error-mesh-unsupported-format = Unsupported file type “.{ $extension }”. Supported formats: glTF (.glb, .gltf), FBX and OBJ.
error-mesh-parse = The { $format } file is invalid: { $message }
error-mesh-no-geometry = The file contains no triangle.
error-mesh-missing-uvs = These objects have no UV coordinates: { $objects }. Unwrap them in your modeling software, then import the mesh again.
error-mesh-too-many-vertices = The mesh has too many vertices ({ $count }).
error-mesh-tangents = Tangent computation failed: { $message }
error-mesh-internal = Internal error while importing the mesh: { $message }. Please report it.

## Display settings panel
display-environment = Environment
display-environment-studio = Studio (generated)
display-load-hdri = Load HDRI…
display-hdri-filter = HDR images
display-loading = Loading…
display-rotation = Rotation
display-exposure = Exposure (EV)
display-tone-mapper = Tone mapping
tone-mapper-agx = AgX
tone-mapper-aces = ACES
tone-mapper-neutral = PBR Neutral
tone-mapper-linear = Linear
display-background = Background
display-background-environment = Environment
display-background-color = Color
display-blur = Blur
display-overlays = Overlays
display-projection = Projection
display-perspective = Perspective
display-orthographic = Orthographic
display-fov = Field of view
error-environment = Cannot load the environment: { $message }

## History
command-rename-texture-set = Rename texture set to “{ $name }”
command-texture-set-resolution = Texture set resolution: { $resolution }
command-environment-source = Change environment
command-environment-rotation = Rotate environment
command-environment-exposure = Exposure
command-add-layer = Add layer “{ $name }”
command-remove-layer = Delete layer “{ $name }”
command-move-layer = Move layer “{ $name }”
command-rename-layer = Rename layer to “{ $name }”
command-show-layer = Show layer “{ $name }”
command-hide-layer = Hide layer “{ $name }”
command-channel-blend = { $channel } blending of “{ $name }”
history-initial = Project opened
history-truncated = Older steps released (memory budget)

## Project files
file-filter = Patine projects
file-busy-saving = Saving…
file-busy-opening = Opening…
file-error-save = Cannot save { $path }: { $message }
file-error-open = Cannot open { $path }: { $message }
file-error-not-a-project = this file is not a Patine project.
file-error-newer-version = this project comes from a newer version of Patine (format { $version }).
file-error-damaged = the file is damaged ({ $message }).
unsaved-title = Unsaved changes
unsaved-message = Save the changes to “{ $name }”?
unsaved-save = Save
unsaved-discard = Don't save
unsaved-cancel = Cancel
error-title = Error
error-ok = OK

## Recovery
recovery-title = Recovery
recovery-message = Patine closed without saving “{ $name }”. A copy saved automatically { $minutes ->
        [0] less than a minute ago
       *[other] { $minutes } min ago
    } can be recovered.
recovery-recover = Recover
recovery-discard = Delete the copy

## Channels
channel-base-color = Base color
channel-metallic = Metallic
channel-roughness = Roughness
channel-normal = Normal
channel-height = Height
channel-ambient-occlusion = Ambient occlusion
channel-emissive = Emissive
channel-opacity = Opacity
channel-user = User { $index }
command-add-channel = Add channel { $channel }
command-remove-channel = Remove channel { $channel }
command-channel-default = Default value: { $channel }

## Display modes
display-mode = Display mode
display-mode-lit = Lit material
display-mode-checker = UV checker
display-mode-mask = Mask of the selected layer
display-mode-channel = Channel: { $channel }
action-view-toggle-channel = Isolated channel
action-view-next-channel = Next channel

## Texture sets panel (channels)
texture-sets-channels = Channels of “{ $name }”
texture-sets-add-channel = Add a channel
texture-sets-remove-channel = Remove this channel
texture-sets-select-hint = Click: current texture set · double-click: rename
texture-sets-memory = GPU memory of the channels: { $document } MB
texture-sets-normal-default = (0, 0, 1)

## 2D view
view-2d-empty = No project open.
view-2d-hint = Wheel: zoom · middle click: pan · F: frame

## Layers panel
layers-empty = No project open.
layers-no-layer = No layer: the stack gives the default values of the channels. Add a fill layer.
layers-new-fill = Fill
layers-new-folder = Folder
layers-new-fill-name = Fill layer { $number }
layers-new-folder-name = Folder { $number }
layers-channel-unused = (channel not affected)
action-layer-delete = Delete layer
action-layer-group = Group into a folder
command-fill-value = Value of “{ $name }”: { $channel }
command-group-layers = Group “{ $name }”
command-activate-channel = Enable { $channel } on “{ $name }”
properties-empty = Select a layer in the Layers panel.
properties-channels = Channels of texture set “{ $set }”

## Blend modes
blend-normal = Normal
blend-replace = Replace
blend-passthrough = Passthrough
blend-multiply = Multiply
blend-divide = Divide
blend-screen = Screen
blend-overlay = Overlay
blend-soft-light = Soft light
blend-hard-light = Hard light
blend-linear-dodge = Linear dodge (add)
blend-linear-burn = Linear burn
blend-subtract = Subtract
blend-difference = Difference
blend-exclusion = Exclusion
blend-darken = Darken
blend-lighten = Lighten
blend-color-dodge = Color dodge
blend-color-burn = Color burn
blend-hue = Hue
blend-saturation = Saturation
blend-color = Color
blend-luminosity = Luminosity
blend-disable = Disabled
blend-normal-map-combine = Normal map combine
blend-normal-map-detail = Normal map detail

## Masks
command-black-mask = Black mask on “{ $name }”
command-white-mask = White mask on “{ $name }”
command-remove-mask = Remove the mask of “{ $name }”
command-add-mask-effect = Add { $effect ->
        [paint] paint
        [fill] a fill
        [levels] a levels
        [invert] an invert
       *[other] an effect
    } to the mask of “{ $name }”
command-remove-mask-effect = Remove an effect from the mask of “{ $name }”
command-move-mask-effect = Move an effect in the mask of “{ $name }”
command-mask-effect = { $effect ->
        [paint] Paint
        [fill] Fill
        [levels] Levels
        [invert] Invert
       *[other] Effect
    } of the mask of “{ $name }”

## Masks in the interface
action-view-toggle-mask = Mask of the selected layer
display-mask-no-layer = Mask: no layer selected
display-mask-of = Mask of “{ $name }”
display-mask-none = “{ $name }” has no mask (fully visible)
mask-add-black = Add a black mask
mask-add-white = Add a white mask
mask-remove = Remove the mask
mask-add-fill = Add a fill
mask-add-levels = Add a levels
mask-add-invert = Add an invert
mask-remove-effect = Remove the effect
mask-row-black = Mask (black)
mask-row-white = Mask (white)
mask-effect-fill = Fill
mask-effect-levels = Levels
mask-effect-invert = Invert
mask-effect-other = Effect
mask-properties = Mask
mask-initial = Initial value
mask-initial-black = Black
mask-initial-white = White
mask-hint = Right-click the mask to add effects to it. M key: show the mask.
mask-visible = Visible
mask-value = Value
mask-levels-in-low = Input black
mask-levels-in-high = Input white
mask-levels-gamma = Gamma
mask-levels-out-low = Output black
mask-levels-out-high = Output white
mask-no-parameter = No parameter.

## Procedurals
command-fill-source = Content of “{ $name }”
command-projection = Projection of “{ $name }”

## Procedural library
library-materials = Materials
library-masks = Masks: noises and patterns
library-materials-hint = Double-click: add as a layer above the selection.
library-masks-hint = Double-click: add to the mask of the selected layer.
mask-add-procedural = Add a procedural fill
procedural-unknown = Procedural missing from the library: this content does not contribute.
properties-projection = Projection
param-random-seed = Random seed
projection-kind = Kind
projection-uv = UV
projection-triplanar = Triplanar
projection-planar = Planar
projection-spherical = Spherical
projection-cylindrical = Cylindrical
projection-blend = Blend
projection-scale = Scale
projection-rotation = Rotation
projection-offset = Offset
procedural-material-steel_brushed = Brushed steel
procedural-material-copper = Copper
procedural-material-plastic_grained = Grained plastic
procedural-material-wood = Wood
procedural-material-concrete = Concrete
procedural-noise-value = Value noise
procedural-noise-perlin = Perlin noise
procedural-noise-simplex = Simplex noise
procedural-noise-worley = Worley noise
procedural-noise-fbm = fBm
procedural-noise-ridged = Ridged noise
procedural-noise-turbulence = Turbulence
procedural-noise-warp = Domain warp
procedural-pattern-gradient = Gradient
procedural-pattern-checker = Checker
procedural-pattern-tiles = Tiles
procedural-pattern-bricks = Bricks
procedural-pattern-stripes = Stripes
procedural-pattern-circles = Circles
procedural-pattern-scratches = Scratches
procedural-pattern-fibers = Fibers
param-color = Color
param-roughness = Roughness
param-scale = Scale
param-brushing = Brushing
param-relief = Relief
param-seed = Seed
param-variation = Variation
param-oxidation = Oxidation
param-grain = Grain
param-light = Light wood
param-dark = Dark wood
param-rings = Rings
param-distortion = Distortion
param-pores = Pores
param-stains = Stains
param-mode = Mode
param-octaves = Octaves
param-gain = Gain
param-strength = Strength
param-invert = Invert
param-count = Count
param-gap = Gap
param-bevel = Bevel
param-width = Width
param-softness = Softness
param-axis = Axis
param-radius = Radius
param-length = Length
param-stretch = Stretch
option-f1 = F1
option-f2 = F2
option-f2-f1 = F2 − F1
option-linear-x = Linear in X
option-linear-y = Linear in Y
option-triangle = Triangle
option-radial = Radial
option-x = X
option-y = Y

## Painting
command-paint-stroke = Stroke on “{ $name }”
command-paint-mask-stroke = Stroke on the mask of “{ $name }”
command-erase-stroke = Erase on “{ $name }”

## Painting in the interface
paint-heading = Brush
tool-brush = Brush
tool-eraser = Eraser
brush-strokes = { $count ->
        [0] No stroke
        [one] One stroke
       *[other] { $count } strokes
    }
brush-size = Size (px)
brush-flow = Flow
brush-opacity = Opacity
brush-hardness = Hardness
brush-spacing = Spacing
brush-angle = Angle (°)
brush-smoothing = Smoothing (px)
brush-alpha = Alpha
alpha-round = Round
alpha-square = Square
alpha-splatter = Splatter
alpha-grain = Grain
alpha-chisel = Chisel
brush-mask-value = Value painted into the mask
brush-channels = Painted channels
brush-normal-flat = flat normal
paint-not-paintable = Select a paint layer, or a paint effect of a mask.
paint-no-channel = The brush paints no channel of this layer.
layers-new-paint = Paint
layers-new-paint-name = Paint layer { $number }
mask-add-paint = Add paint
mask-effect-paint = Paint
action-tool-brush = Brush
action-tool-eraser = Eraser
action-brush-smaller = Smaller brush
action-brush-larger = Larger brush
action-brush-softer = Softer brush
action-brush-harder = Harder brush
action-layer-new-paint = New paint layer
menu-paint = Paint
command-height-force = Height force

## Painting v2
preset-standard = Standard
preset-airbrush = Airbrush
preset-hard-felt = Hard felt
preset-chalk = Chalk
preset-splatter = Splatter
preset-scratches = Scratches
brush-preset = Preset
brush-preset-none = Custom
brush-preset-name = Name
brush-preset-save = Save
brush-preset-delete = Delete
brush-preset-error = Preset not saved: { $error }
brush-dynamics = Dynamics
brush-size-jitter = Size jitter
brush-angle-jitter = Angle jitter (°)
brush-flow-jitter = Flow jitter
brush-scatter = Scatter
brush-follow-path = Follow the path
brush-tilt-angle = Pen tilt turns the dabs
brush-pressure = Pen pressure
pressure-size = Size
pressure-flow = Flow
pressure-opacity = Opacity
pressure-curve-reset = Linear
pressure-curve-off = Off
pressure-curve-hint = Drag the points; double-click to add one, right-click to remove it.
brush-symmetry = Symmetry
symmetry-mirror = Mirror
symmetry-radial = Radial copies
symmetry-limit = At most { $max } copies: the extra ones are not painted.
symmetry-2d = Symmetry does not apply in the 2D view.
brush-material = Material
brush-material-values = Values
brush-material-procedural = Procedural
brush-mask-pattern = Procedural pattern
tool-picker = Picker
action-tool-picker = Picker
action-symmetry-x = Mirror X symmetry
picker-nothing = Nothing to pick here.
paint-line-hint = Shift + click: straight line from the end of the last stroke
texture-sets-height-force = Height force
