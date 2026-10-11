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
display-material = Preview material
display-base-color = Base color
display-metallic = Metallic
display-roughness = Roughness
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
