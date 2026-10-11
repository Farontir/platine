# Patine — textes en français.
# Chaque clé doit aussi exister dans i18n/en/patine.ftl (vérifié par un test).

app-title = Patine

## Menus
menu-file = Fichier
menu-edit = Édition
menu-view = Affichage
menu-language = Langue
menu-help = Aide

## Actions
action-view-frame = Cadrer le mesh
action-view-orthographic = Orthographique
action-view-wireframe = Fil de fer
action-view-shadows = Ombres
action-file-new-project = Nouveau projet…
action-file-open = Ouvrir…
action-file-save = Enregistrer
action-file-save-as = Enregistrer sous…
action-app-quit = Quitter
action-view-reset-layout = Réinitialiser la disposition
action-debug-toggle-profiler = Profileur
action-language-english = English
action-language-french = Français
action-help-about = À propos de Patine
action-edit-undo = Annuler
action-edit-redo = Rétablir

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
status-hover = Triangle { $triangle } · polygone { $polygon } · { $object }

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
texture-sets-rename-hint = Double-cliquez pour renommer

## Mesh import errors
error-mesh-io = Impossible de lire { $path } : { $message }
error-mesh-unsupported-format = Type de fichier « .{ $extension } » non pris en charge. Formats acceptés : glTF (.glb, .gltf), FBX et OBJ.
error-mesh-parse = Le fichier { $format } est invalide : { $message }
error-mesh-no-geometry = Le fichier ne contient aucun triangle.
error-mesh-missing-uvs = Ces objets n'ont pas de coordonnées UV : { $objects }. Dépliez-les dans votre logiciel de modélisation, puis importez à nouveau le mesh.
error-mesh-too-many-vertices = Le mesh a trop de sommets ({ $count }).
error-mesh-tangents = Le calcul des tangentes a échoué : { $message }
error-mesh-internal = Erreur interne pendant l'import du mesh : { $message }. Merci de la signaler.

## Display settings panel
display-environment = Environnement
display-environment-studio = Studio (généré)
display-load-hdri = Charger une HDRI…
display-hdri-filter = Images HDR
display-loading = Chargement…
display-rotation = Rotation
display-exposure = Exposition (EV)
display-tone-mapper = Tone mapping
tone-mapper-agx = AgX
tone-mapper-aces = ACES
tone-mapper-neutral = PBR Neutral
tone-mapper-linear = Linéaire
display-background = Fond
display-background-environment = Environnement
display-background-color = Couleur
display-blur = Flou
display-overlays = Superpositions
display-projection = Projection
display-perspective = Perspective
display-orthographic = Orthographique
display-fov = Champ de vision
error-environment = Impossible de charger l'environnement : { $message }

## History
command-rename-texture-set = Renommer le texture set en « { $name } »
command-texture-set-resolution = Résolution du texture set : { $resolution }
command-environment-source = Changer d'environnement
command-environment-rotation = Rotation de l'environnement
command-environment-exposure = Exposition
command-add-layer = Ajouter le calque « { $name } »
command-remove-layer = Supprimer le calque « { $name } »
command-move-layer = Déplacer le calque « { $name } »
command-rename-layer = Renommer le calque en « { $name } »
command-show-layer = Afficher le calque « { $name } »
command-hide-layer = Masquer le calque « { $name } »
command-channel-blend = Mélange du canal { $channel } de « { $name } »
history-initial = Ouverture du projet
history-truncated = Étapes plus anciennes libérées (budget mémoire)

## Project files
file-filter = Projets Patine
file-busy-saving = Enregistrement…
file-busy-opening = Ouverture…
file-error-save = Impossible d'enregistrer { $path } : { $message }
file-error-open = Impossible d'ouvrir { $path } : { $message }
file-error-not-a-project = ce fichier n'est pas un projet Patine.
file-error-newer-version = ce projet vient d'une version plus récente de Patine (format { $version }).
file-error-damaged = le fichier est endommagé ({ $message }).
unsaved-title = Modifications non enregistrées
unsaved-message = Enregistrer les modifications de « { $name } » ?
unsaved-save = Enregistrer
unsaved-discard = Ne pas enregistrer
unsaved-cancel = Annuler
error-title = Erreur
error-ok = OK

## Recovery
recovery-title = Récupération
recovery-message = Patine s'est fermé sans enregistrer « { $name } ». Une copie enregistrée automatiquement { $minutes ->
        [0] il y a moins d'une minute
       *[other] il y a { $minutes } min
    } peut être récupérée.
recovery-recover = Récupérer
recovery-discard = Supprimer la copie

## Channels
channel-base-color = Couleur de base
channel-metallic = Metallic
channel-roughness = Roughness
channel-normal = Normale
channel-height = Hauteur
channel-ambient-occlusion = Occlusion ambiante
channel-emissive = Émissif
channel-opacity = Opacité
channel-user = Utilisateur { $index }
command-add-channel = Ajouter le canal { $channel }
command-remove-channel = Retirer le canal { $channel }
command-channel-default = Valeur par défaut : { $channel }

## Display modes
display-mode = Mode d'affichage
display-mode-lit = Matériau éclairé
display-mode-checker = Damier UV
display-mode-mask = Masque du calque sélectionné
display-mode-channel = Canal : { $channel }
action-view-toggle-channel = Canal isolé
action-view-next-channel = Canal suivant

## Texture sets panel (channels)
texture-sets-channels = Canaux de « { $name } »
texture-sets-add-channel = Ajouter un canal
texture-sets-remove-channel = Retirer ce canal
texture-sets-select-hint = Clic : texture set courant · double-clic : renommer
texture-sets-memory = Mémoire GPU des canaux : { $document } Mo
texture-sets-normal-default = (0, 0, 1)

## 2D view
view-2d-empty = Aucun projet ouvert.
view-2d-hint = Molette : zoom · clic milieu : déplacer · F : cadrer

## Layers panel
layers-empty = Aucun projet ouvert.
layers-no-layer = Aucun calque : la pile donne les valeurs par défaut des canaux. Ajoutez un calque fill.
layers-new-fill = Fill
layers-new-folder = Dossier
layers-new-fill-name = Calque fill { $number }
layers-new-folder-name = Dossier { $number }
layers-channel-unused = (canal non affecté)
action-layer-delete = Supprimer le calque
action-layer-group = Grouper dans un dossier
command-fill-value = Valeur de « { $name } » : { $channel }
command-group-layers = Grouper « { $name } »
command-activate-channel = Activer { $channel } sur « { $name } »
properties-empty = Sélectionnez un calque dans le panneau Calques.
properties-channels = Canaux du texture set « { $set } »

## Blend modes
blend-normal = Normal
blend-replace = Remplacer
blend-passthrough = Transparent (passthrough)
blend-multiply = Produit
blend-divide = Diviser
blend-screen = Superposition écran
blend-overlay = Incrustation
blend-soft-light = Lumière tamisée
blend-hard-light = Lumière crue
blend-linear-dodge = Addition (densité linéaire -)
blend-linear-burn = Densité linéaire +
blend-subtract = Soustraction
blend-difference = Différence
blend-exclusion = Exclusion
blend-darken = Obscurcir
blend-lighten = Éclaircir
blend-color-dodge = Densité couleur -
blend-color-burn = Densité couleur +
blend-hue = Teinte
blend-saturation = Saturation
blend-color = Couleur
blend-luminosity = Luminosité
blend-disable = Désactivé
blend-normal-map-combine = Combiner les normales
blend-normal-map-detail = Détail de normale

## Masks
command-black-mask = Masque noir sur « { $name } »
command-white-mask = Masque blanc sur « { $name } »
command-remove-mask = Supprimer le masque de « { $name } »
command-add-mask-effect = Ajouter { $effect ->
        [fill] un fill
        [levels] un levels
        [invert] une inversion
       *[other] un effet
    } au masque de « { $name } »
command-remove-mask-effect = Supprimer un effet du masque de « { $name } »
command-move-mask-effect = Déplacer un effet du masque de « { $name } »
command-mask-effect = { $effect ->
        [fill] Fill
        [levels] Levels
        [invert] Inversion
       *[other] Effet
    } du masque de « { $name } »

## Masks in the interface
action-view-toggle-mask = Masque du calque sélectionné
display-mask-no-layer = Masque : aucun calque sélectionné
display-mask-of = Masque de « { $name } »
display-mask-none = « { $name } » n'a pas de masque (tout visible)
mask-add-black = Ajouter un masque noir
mask-add-white = Ajouter un masque blanc
mask-remove = Supprimer le masque
mask-add-fill = Ajouter un fill
mask-add-levels = Ajouter un levels
mask-add-invert = Ajouter une inversion
mask-remove-effect = Supprimer l'effet
mask-row-black = Masque (noir)
mask-row-white = Masque (blanc)
mask-effect-fill = Fill
mask-effect-levels = Levels
mask-effect-invert = Inversion
mask-effect-other = Effet
mask-properties = Masque
mask-initial = Valeur initiale
mask-initial-black = Noir
mask-initial-white = Blanc
mask-hint = Clic droit sur le masque pour lui ajouter des effets. Touche M : afficher le masque.
mask-visible = Visible
mask-value = Valeur
mask-levels-in-low = Noir d'entrée
mask-levels-in-high = Blanc d'entrée
mask-levels-gamma = Gamma
mask-levels-out-low = Noir de sortie
mask-levels-out-high = Blanc de sortie
mask-no-parameter = Pas de paramètre.
