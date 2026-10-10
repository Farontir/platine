# Patine — English strings.
# Every key must also exist in i18n/fr/patine.ftl (checked by a test).

app-title = Patine

## Menus
menu-file = File
menu-view = View
menu-language = Language
menu-help = Help

## Actions
action-file-new-project = New project…
action-app-quit = Quit
action-view-reset-layout = Reset layout
action-debug-toggle-profiler = Profiler
action-language-english = English
action-language-french = Français
action-help-about = About Patine

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

## Mesh import errors
error-mesh-io = Cannot read { $path }: { $message }
error-mesh-unsupported-format = Unsupported file type “.{ $extension }”. Supported formats: glTF (.glb, .gltf), FBX and OBJ.
error-mesh-parse = The { $format } file is invalid: { $message }
error-mesh-no-geometry = The file contains no triangle.
error-mesh-missing-uvs = These objects have no UV coordinates: { $objects }. Unwrap them in your modeling software, then import the mesh again.
error-mesh-too-many-vertices = The mesh has too many vertices ({ $count }).
error-mesh-tangents = Tangent computation failed: { $message }
error-mesh-internal = Internal error while importing the mesh: { $message }. Please report it.
