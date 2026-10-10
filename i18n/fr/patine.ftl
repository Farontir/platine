# Patine — textes en français.
# Chaque clé doit aussi exister dans i18n/en/patine.ftl (vérifié par un test).

app-title = Patine

## Menus
menu-file = Fichier
menu-view = Affichage
menu-language = Langue
menu-help = Aide

## Actions
action-file-new-project = Nouveau projet…
action-app-quit = Quitter
action-view-reset-layout = Réinitialiser la disposition
action-debug-toggle-profiler = Profileur
action-language-english = English
action-language-french = Français
action-help-about = À propos de Patine

## Panels
panel-viewport-3d = Vue 3D
panel-view-2d = Vue 2D
panel-texture-sets = Texture sets
panel-layers = Calques
panel-properties = Propriétés
panel-library = Bibliothèque
panel-display = Paramètres d'affichage
panel-history = Historique
panel-console = Console
panel-empty = Rien ici pour l'instant.

## Console
console-clear = Effacer
console-level = Niveau minimal
console-empty = Aucun message.
console-level-error = Erreurs
console-level-warn = Avertissements
console-level-info = Informations
console-level-debug = Débogage
console-level-trace = Trace
console-profiler-on = Profileur actif : connectez puffin_viewer à 127.0.0.1:{ $port }

## About
about-description = Texturing PBR par calques sur mesh 3D.
about-version = Version { $version }
about-close = Fermer

## Status bar
status-gpu = GPU : { $name } ({ $backend })
status-project = Projet : { $name }

## New project
new-project-title = Nouveau projet
new-project-mesh = Mesh
new-project-browse = Parcourir…
new-project-filter = Meshes 3D
new-project-resolution = Résolution
new-project-normal-convention = Normal map
normal-convention-opengl = OpenGL (Y+)
normal-convention-directx = DirectX (Y−)
new-project-create = Créer
new-project-cancel = Annuler
new-project-importing = Import de { $file }…

## Texture sets panel
texture-sets-empty = Aucun projet pour l'instant. Créez-en un avec Fichier > Nouveau projet (Ctrl+N).
texture-sets-mesh = { $name } : { $triangles } triangles, { $vertices } sommets
texture-sets-name = Texture set
texture-sets-resolution = Résolution
texture-sets-triangles = Triangles

## Mesh import errors
error-mesh-io = Impossible de lire { $path } : { $message }
error-mesh-unsupported-format = Type de fichier « .{ $extension } » non pris en charge. Formats acceptés : glTF (.glb, .gltf), FBX et OBJ.
error-mesh-parse = Le fichier { $format } est invalide : { $message }
error-mesh-no-geometry = Le fichier ne contient aucun triangle.
error-mesh-missing-uvs = Ces objets n'ont pas de coordonnées UV : { $objects }. Dépliez-les dans votre logiciel de modélisation, puis importez à nouveau le mesh.
error-mesh-too-many-vertices = Le mesh a trop de sommets ({ $count }).
error-mesh-tangents = Le calcul des tangentes a échoué : { $message }
error-mesh-internal = Erreur interne pendant l'import du mesh : { $message }. Merci de la signaler.
