# Plan d'implémentation — Patine (équivalent de Substance Painter)

Oct 10, 2026 · @Jeremy Delbarre

## Comment utiliser ce plan avec Claude Code

Ce plan se donne à Claude Code un milestone à la fois : il sert de source de vérité pour la stack, l'architecture et les critères d'acceptation, et Claude Code avance session par session. « Patine » est un nom de code ; pour le changer, il suffit de remplacer le préfixe `patine_` partout.

1. Crée un dépôt Git vide et place ce document exporté en Markdown dans `docs/PLAN.md`.
2. Crée `CLAUDE.md` à la racine avec le contenu de la section « Contenu prêt à copier dans CLAUDE.md ».
3. Lance la première session avec le prompt de la dernière section. Elle couvre le milestone M0.
4. Une session correspond à un milestone, ou à la moitié d'un gros. Claude Code commence en plan mode, propose son découpage, tu valides, puis il implémente, teste, coche les cases de `docs/PLAN.md` et commit.
5. Entre deux milestones, teste l'application à la main : la qualité d'un trait de pinceau ou d'un rendu ne se voit pas dans les tests automatiques.
6. Si un choix technique change, mets d'abord `docs/PLAN.md` à jour, puis le code. Sinon, les sessions suivantes repartent sur l'ancienne décision.

Compte plusieurs dizaines de sessions pour atteindre le MVP. Claude Code n'écrira pas ce logiciel d'une traite, et la qualité finale dépendra surtout de tes tests manuels entre les milestones.

## Vision, périmètre et contraintes légales

Patine reproduit le workflow complet de Substance Painter, c'est-à-dire le texturing PBR par calques directement sur un mesh 3D, avec ses propres noms, assets et formats.

### Périmètre du MVP

- Import de meshes glTF, OBJ et FBX, avec un texture set par matériau.
- Viewport PBR temps réel éclairé par HDRI.
- Pile de calques : paint, fill, dossiers et instances, avec blend mode et opacité par canal.
- Masques : peints, fill, générateurs, filtres, anchor points et polygon fill.
- Peinture 3D projetée : brosses, alphas, symétrie, pression tablette.
- Indépendance de résolution grâce aux strokes rejouables.
- Baker high poly vers low poly : normal, world normal, AO, courbure, position, thickness, ID et height.
- Matériaux et générateurs procéduraux, smart materials, smart masks et bibliothèque d'assets.
- Export avec presets Unreal, Unity, Godot et glTF.
- Sauvegarde, autosave, undo/redo borné en mémoire.

### Non-objectifs

Pas d'équivalent de Designer, donc pas d'éditeur de nœuds exposé à l'utilisateur. Pas de modélisation, de sculpt ni de dépliage UV : le mesh arrive avec ses UV. Pas d'animation, et aucune compatibilité avec les fichiers Adobe.

### Plateformes

Windows 10/11 d'abord, sur la machine de développement (RTX 3080 Ti) et le portable (RTX A2000) qui sert de référence « machine modeste ». Linux ensuite. macOS après le MVP : wgpu passe par Metal sans réécriture.

### Contraintes légales, à respecter strictement

- Les mots « Substance » et « Adobe » n'apparaissent nulle part dans le produit : ni dans l'interface, ni dans le code, ni dans la documentation utilisateur. Les termes génériques (layer, fill, mask, anchor, smart material) restent libres.
- Aucun asset Adobe : ni matériaux, ni brosses, ni alphas, ni icônes, ni captures d'écran.
- Aucune lecture ni écriture des formats propriétaires (.spp, .sbsar, .sbs, .sppr). Formats ouverts uniquement.
- L'organisation fonctionnelle de l'interface peut être proche, mais les icônes, le thème et les visuels sont originaux.
- Dépendances sous licences permissives uniquement (MIT, Apache-2.0, BSD, zlib), vérifiées par `cargo-deny` en CI.
- Assets de test et de bibliothèque : CC0 (Poly Haven, ambientCG) ou générés par le projet.

Ce cadrage limite les risques, mais il ne remplace pas l'avis d'un juriste avant une commercialisation.

## Stack technique imposée

Tout est en Rust avec wgpu : une seule base de code pour Vulkan, DirectX 12, Metal et WebGPU, et des compute shaders pour tout le traitement d'images. Claude Code n'ajoute aucune autre dépendance sans la justifier dans le message de commit. Au M0, il vérifie la dernière version stable de chaque crate et la fige dans `Cargo.lock`.

| Besoin | Choix | Raison |
| --- | --- | --- |
| Langage | Rust stable, édition 2024 | Sécurité mémoire, performances natives |
| API GPU | wgpu | Vulkan, DX12, Metal et WebGPU depuis une seule API |
| Shaders | WGSL, validés par naga | Langage natif de wgpu, validation possible en test |
| Fenêtre et entrées | winit | Standard de l'écosystème |
| Interface | egui, egui-wgpu, egui\_dock | Rapide à écrire, panneaux ancrables |
| Maths | glam | Types SIMD, standard du gamedev Rust |
| Import glTF | gltf | Crate mature |
| Import FBX et OBJ | ufbx (bindings Rust) | Parser open source sous licence MIT, sans SDK Autodesk ; triangule les n-gones concaves et lit les .mtl (remplace tobj, décidé au M1) |
| Tangentes | bevy\_mikktspace | Normal maps identiques à Blender, Unity et Unreal ; mikktspace en Rust sûr |
| Images | image, exr | PNG, TGA, TIFF, JPEG, HDR et OpenEXR |
| Tablette graphique | octotablet (à valider au M9) | Pression et inclinaison du stylet |
| Parallélisme CPU | rayon | Baker CPU, encodage d'export |
| BVH | Implémentation maison (SAH) | Même disposition mémoire côté CPU et GPU |
| Sérialisation | serde, RON, postcard, zstd, zip | Manifeste lisible, données binaires compressées (bincode abandonné : la 3.0 est un tombstone) |
| Dialogues fichiers | rfd | Dialogues natifs |
| Erreurs | thiserror (crates), anyhow (application) | Erreurs typées dans les bibliothèques |
| Logs et profilage | tracing, puffin, puffin\_http | Profileur activable dans l'application, affiché dans puffin\_viewer (puffin\_egui ne suit pas encore egui 0.36) |
| Tests | cargo test, proptest, criterion | Tests unitaires, propriétés, benchmarks |
| CI | GitHub Actions, cargo-deny | fmt, clippy, tests GPU logiciels, licences |

## Structure du dépôt

Le dépôt est un workspace Cargo découpé en crates par sous-système, pour que chaque milestone touche un périmètre clair et que le cœur se teste sans GPU.

```
patine/
├── Cargo.toml              # workspace
├── CLAUDE.md
├── deny.toml
├── docs/
│   ├── PLAN.md
│   └── design/             # une note de conception par milestone (M05-compositor.md, ...)
├── crates/
│   ├── patine_core/        # modèle de document, commandes, undo, sérialisation (sans wgpu)
│   ├── patine_mesh/        # import, tangentes, adjacence, îlots UV, BVH
│   ├── patine_gpu/         # contexte wgpu, pool de textures, budget mémoire, shaders
│   ├── patine_render/      # viewport PBR, IBL, tone mapping, overlays
│   ├── patine_compositor/  # évaluation de la pile de calques sur GPU
│   ├── patine_procedural/  # bruits, motifs, générateurs, filtres
│   ├── patine_paint/       # moteur de brosses, projection, strokes
│   ├── patine_bake/        # baker CPU de référence + baker GPU
│   ├── patine_export/      # presets, packing, écriture des fichiers
│   ├── patine_assets/      # bibliothèque, presets, vignettes
│   ├── patine_app/         # binaire : interface egui, outils, raccourcis
│   └── patine_cli/         # binaire headless : bake et export en ligne de commande
├── shaders/                # tous les .wgsl, rangés par crate
├── assets/                 # bibliothèque par défaut (CC0 ou générée)
└── tests/
    ├── fixtures/           # meshes et HDRI de test CC0
    └── golden/             # images de référence des tests de rendu
```

### Règles de dépendances

- `patine_core` ne dépend ni de wgpu ni d'egui. Tout le modèle de document se teste sans GPU.
- Les crates GPU dépendent de `patine_core` et `patine_gpu`, jamais de `patine_app`.
- `patine_cli` n'utilise aucune dépendance d'interface. S'il sait baker et exporter un projet, la logique est bien séparée de l'UI.
- Les shaders sont intégrés au binaire avec `include_str!` en release. En debug, la feature `hot-reload` les recharge depuis le disque sans recompiler.

## Architecture et modèle de données

Le document est une donnée pure côté CPU, et le GPU n'en est qu'un cache recalculable. C'est ce principe qui rend possibles l'undo, la sauvegarde et l'indépendance de résolution.

Le flux est le suivant : le mesh importé passe par le baker, qui produit les baked maps. Le compositeur combine ces maps avec la pile de calques du document, y compris les strokes rejoués, pour produire les textures finales. Le viewport et l'exporteur lisent ces textures finales.

&#91;embedded content: architecture · 7 sous-systèmes autour du document\]

L'interface ne touche jamais aux textures : elle émet des commandes sur le document, et le compositeur en déduit les textures finales.

### Modèle de données

```rust
struct Project {
    meshes: Vec<MeshAsset>,
    texture_sets: Vec<TextureSet>,     // un par matériau du mesh
    environment: EnvironmentSettings,  // HDRI, rotation, exposition
    export_presets: Vec<ExportPreset>,
}

struct TextureSet {
    id: TextureSetId,
    name: String,
    material_index: u32,
    resolution: Resolution,            // 512 à 8192, puissance de 2
    channels: Vec<ChannelDef>,         // canaux actifs et leur format
    baked_maps: BakedMaps,
    stack: LayerStack,
}

enum Channel {
    BaseColor, Metallic, Roughness, Normal, Height,
    AmbientOcclusion, Emissive, Opacity, User(u8),
}

struct Layer {
    id: LayerId,                       // stable, jamais réutilisé
    name: String,
    visible: bool,
    kind: LayerKind,
    channels: ChannelMap<ChannelBlend>, // blend mode + opacité par canal
    mask: Option<MaskStack>,
    effects: Vec<Effect>,              // filtres et niveaux appliqués au contenu
    anchor: Option<AnchorId>,          // expose le résultat intermédiaire
}

enum LayerKind {
    Paint { strokes: StrokeLog },
    Fill { source: FillSource, projection: Projection }, // valeur, image ou matériau procédural
    Folder { children: Vec<Layer> },
    Instance { source: LayerId },      // calque partagé entre texture sets
}

enum MaskEffect {
    Paint { strokes: StrokeLog },
    Fill { source: FillSource },
    Generator { kind: GeneratorKind, params: Params },
    Filter { kind: FilterKind, params: Params },
    Levels(LevelsParams),
    AnchorRef { anchor: AnchorId, channel: Channel },
    PolygonFill { selection: FaceSelection },
}
```

Chaque `MaskEffect` porte aussi son blend mode et son opacité. Les identifiants sont stables pour toute la vie du projet : l'undo, les instances et les anchor points s'appuient dessus.

### Indépendance de résolution

- Un calque paint stocke un `StrokeLog`. Chaque stroke contient un instantané du preset de brosse, l'outil, la caméra au moment du trait (matrice view-projection, taille du viewport), la symétrie et la liste des dabs (position écran, pression, inclinaison, horodatage).
- La rastérisation rejoue les strokes en reprojetant chaque dab sur le mesh, à n'importe quelle résolution. Le jitter aléatoire utilise une graine stockée dans le stroke pour que le rejeu soit déterministe.
- Les calques fill et procéduraux sont simplement réévalués dans l'espace UV à la résolution courante.
- Règle absolue : aucune donnée du document n'existe uniquement sous forme de pixels, sauf les images importées par l'utilisateur, référencées par leur hash.

### Commandes et undo

- Toute modification passe par une `Command` avec `apply` et `revert`. L'interface ne modifie jamais le document directement.
- Une commande de stroke garde le stroke lui-même, plus des instantanés des tuiles 128×128 touchées dans le cache raster. L'undo est ainsi instantané, sans rejouer toute la pile.
- Les commandes continues fusionnent : faire glisser un slider produit une seule entrée d'historique.
- L'historique a un budget mémoire configurable (2 Go par défaut). Les entrées les plus anciennes sont évincées en premier.

### Document et GPU

- `GpuDocument` reflète le document : textures des calques, caches de composition, baked maps. Un calque n'alloue que les canaux qu'il modifie.
- Formats : `Rgba16Float` pour les couleurs et les normales (espace tangent), `R16Float` pour les canaux scalaires et les masques.
- Les commandes marquent des zones sales. Un calque modifié invalide ses canaux, la composition à partir de sa position et tout ce qui dépend de ses anchor points.
- Ordres de grandeur : une texture 4096² coûte 32 Mo en `R16Float` et 128 Mo en `Rgba16Float`. Quarante calques en 4K dépassent vite les 12 Go de la RTX 3080 Ti, et encore plus la mémoire de la RTX A2000. Le pool de textures évince donc en mémoire CPU compressée (LRU) les calques éloignés de celui qu'on édite.

### Format de projet `.patine`

- Archive zip contenant `manifest.ron` (version et document sans pixels), `meshes/` (copie binaire des meshes importés), `strokes/` (postcard + zstd), `resources/` (images importées, nommées par hash), `cache/` (caches raster EXR facultatifs), `baked/` (baked maps en EXR) et `thumb.png`.
- Un champ `format_version` et une chaîne de migrations `migrate_vN_to_vN1`. Des fichiers fixtures de chaque ancienne version sont rechargés en test.
- Autosave toutes les 5 minutes dans un fichier séparé, et proposition de récupération au démarrage après un crash.

## Viewport PBR

Le viewport doit montrer ce que l'utilisateur obtiendra dans son moteur de jeu. Il utilise donc le shading standard metal/roughness, un éclairage par image (IBL) et un tone mapping configurable.

- **Shading** : Cook-Torrance avec distribution GGX, visibilité Smith height-correlated et Fresnel de Schlick. F0 vaut 0,04 pour les diélectriques.
- **Normal maps** : appliquer exactement les règles de mikktspace (bitangente = signe × cross(N, T), vecteurs interpolés non renormalisés). Sans ça, les normal maps du baker ne correspondent pas au rendu.
- **Height** : convertie en normal de détail par un filtre de Sobel, puis combinée au canal normal. Le parallax occlusion mapping vient après le MVP.
- **IBL** : HDRI équirectangulaire (.hdr ou .exr) convertie en cubemap par compute shader, irradiance en harmoniques sphériques d'ordre 2, chaîne de mips spéculaire préfiltrée GGX, LUT de BRDF 2D. Réglages : rotation, exposition, fond flouté ou couleur unie.
- **Ombres** : une lumière directionnelle alignée sur la direction dominante de l'HDRI, avec une shadow map. Désactivable.
- **AO** : le canal AO n'atténue que l'éclairage indirect.
- **Tone mapping** : AgX par défaut, ACES (approximation), Khronos PBR Neutral, linéaire. Exposition en EV.
- **Couleur** : tout le calcul se fait en linéaire. La surface d'affichage est en sRGB.
- **Anti-aliasing** : MSAA 4x. Le TAA vient après le MVP.
- **Modes d'affichage** : matériau éclairé, canal isolé (chaque canal), masque du calque sélectionné, baked map isolée, damier UV, wireframe en surimpression.
- **Vue 2D** : panneau séparé qui affiche la texture du texture set avec le wireframe des UV par-dessus.
- **Caméra** : orbite, panoramique et zoom à la souris (Alt + boutons, molette), cadrage sur F, perspective ou orthographique, FOV réglable.
- **Opacité** : prévisualisation en alpha blend ou alpha test.
- **Picking** : un buffer d'ID GPU (index de triangle + profondeur) donne le point du mesh sous le curseur. Le moteur de peinture et le polygon fill s'en servent.
- **Curseur de brosse** : cercle projeté sur la surface, orienté selon la normale, à la taille réelle de la brosse.

## Compositeur de calques GPU

Le compositeur évalue la pile de bas en haut, canal par canal, entièrement en compute shaders, et ne recalcule que ce qui a changé. C'est le cœur des performances du logiciel.

### Évaluation

1. Construire un graphe d'évaluation à partir de la pile. Les nœuds sont les contenus de calques, les masques, les effets et les anchor points. Les arêtes sont l'ordre de la pile et les références d'anchor. Un anchor ne peut être référencé que par un calque situé au-dessus de lui, ce qui exclut les cycles dès la commande.
2. Pour chaque canal, partir de la valeur par défaut du `ChannelDef`. Puis, pour chaque calque visible : contenu C, masque M (la pile de `MaskEffect` réduite à une texture `R16Float`), opacité α du canal. On calcule `dst = mix(dst, blend(dst, C), M × α)`.
3. Un dossier évalue ses enfants dans une texture temporaire, puis se mélange comme un seul calque avec son propre masque. Le mode passthrough mélange directement les enfants dans la pile parente.
4. La normale finale combine la normal map bakée (si l'option « utiliser la normale bakée comme base » est active), le canal normal et la normale dérivée du canal height.

### Blend modes par canal

Normal, Replace, Passthrough (dossiers), Multiply, Divide, Screen, Overlay, Soft Light, Hard Light, Linear Dodge (Add), Linear Burn, Subtract, Difference, Exclusion, Darken, Lighten, Color Dodge, Color Burn, Hue, Saturation, Color, Luminosity, Disable. Pour le canal normal : Normal Map Combine (Reoriented Normal Mapping) par défaut, et Normal Map Detail (UDN).

### Caches

- **Contenu** : le contenu et le masque de chaque calque sont mis en cache. Ils ne sont régénérés que si leurs paramètres changent. Un mélange ne coûte alors qu'une passe par calque et par canal.
- **Cache inférieur** : pendant l'édition d'un calque, la composition de tout ce qui est en dessous est figée. Chaque mise à jour ne refait que le calque actif et les mélanges au-dessus.
- **Tuiles sales** : un stroke salit un ensemble de tuiles 128×128. Les passes de mélange ne s'exécutent que sur ces tuiles, via une liste de tuiles passée au dispatch.

### Anchor points et instances

- Un anchor posé sur un calque L capture la composition des canaux choisis juste après L. Un `MaskEffect::AnchorRef` plus haut lit cette texture. Toute modification à L ou en dessous invalide les dépendants.
- Un calque instance rejoue la définition du calque source dans le texture set cible. Il réutilise directement les textures du source quand la résolution et le texture set sont identiques.

### Précision et vérification

Stockage en f16, calculs en f32. Chaque blend mode a une implémentation CPU de référence dans `patine_compositor::reference`. Les tests comparent la sortie GPU à cette référence avec une tolérance de 1/512.

## Moteur de peinture

L'utilisateur peint à l'écran, et chaque dab est reprojeté dans l'espace UV du mesh. La brosse peint un matériau complet, c'est-à-dire tous les canaux activés en même temps, et pas seulement une couleur.

### Règle de conception

Une seule fonction `rasterize_dabs(stroke, dabs, resolution)` rastérise les strokes. La peinture en direct l'appelle sur les nouveaux dabs, et le rejeu l'appelle sur tout le `StrokeLog`. Le rejeu produit donc exactement ce que l'utilisateur a vu en peignant.

### Pipeline d'un dab

1. Les événements du pointeur passent par un échantillonneur : interpolation selon l'espacement (en % de la taille), lissage optionnel (lazy mouse), courbe de pression.
2. Chaque dab est défini en espace écran : centre, taille, rotation, alpha (texture en niveaux de gris), dureté.
3. Seuls les triangles dont la boîte englobante écran touche le dab sont gardés, via le BVH. Le mesh est ensuite rendu en espace UV : le vertex shader émet la position `uv × 2 − 1`.
4. Le fragment shader projette la position monde du texel avec la caméra du stroke, lit l'alpha du dab à cet endroit et teste l'occlusion contre la profondeur capturée au début du stroke. Il atténue aussi selon l'angle entre la normale et la vue.
5. Les dabs s'accumulent dans un buffer de stroke (flow), composé sur le calque avec l'opacité du stroke. L'opacité ne s'accumule ainsi pas à l'intérieur d'un même trait.
6. Comme la rastérisation part des positions 3D, les deux côtés d'une couture UV reçoivent la peinture. Un padding par dilatation est appliqué à la composition et à l'export, sans être stocké dans le calque.

La rastérisation conservative de wgpu n'est pas disponible partout. Il faut donc tester qu'aucun trou n'apparaît aux arêtes des triangles, et sinon élargir légèrement les triangles en UV.

### Paramètres de brosse

Taille (en pixels écran ou en espace objet), flow, opacité, dureté, espacement, angle, jitter de taille, d'angle et de flow, scatter, suivi de la trajectoire, alpha. Courbes de pression vers la taille, le flow et l'opacité, et inclinaison vers l'angle. Le matériau peint est soit une valeur par canal, soit un matériau procédural échantillonné en triplanar : on peint de la rouille directement. Un preset de brosse est une struct sérialisable.

### Outils

- **Brush** et **Eraser** (réduit la couverture du calque canal par canal).
- **Projection** : une image ou un matériau fixé dans l'espace écran, révélé par la brosse. Les stencils fonctionnent de la même façon.
- **Polygon fill** : sélection par triangle, face, îlot UV, partie de mesh ou couleur d'ID map, grâce au picking et à l'adjacence du mesh. La sélection est rastérisée dans un masque.
- **Clone** et **Smudge** : ils lisent la composition courante rendue dans un buffer écran au début du stroke.
- **Ligne droite** (Maj + clic) et **pipette** (lit la composition sous le curseur, canal par canal).
- **Peinture UV** : peindre directement dans la vue 2D.

### Symétrie

Plans X, Y et Z en espace objet, plus un mode radial. Chaque dab est dupliqué avec une caméra miroir. Cette caméra est stockée dans le stroke pour que le rejeu reste exact.

### Entrées

La souris donne une pression de 1. La tablette graphique passe par octotablet, à valider tôt au M9, avec repli sur la souris si la crate ne suffit pas.

## Baker

Le baker transfère le détail d'un mesh high poly vers les UV du low poly. Il existe en deux versions : une référence CPU simple et lente, puis une version GPU qui doit produire le même résultat à la tolérance près.

| Map | Méthode | Format de travail |
| --- | --- | --- |
| Normal (tangent space) | Rayon depuis le low poly, normale du high exprimée dans la base mikktspace du low | `Rgba16Float` |
| World space normal | Normale du high en espace objet | `Rgba16Float` |
| Ambient occlusion | Rayons hémisphériques pondérés par le cosinus, distance max | `R16Float` |
| Curvature | Variation de la world normal entre texels voisins, coutures UV comprises ; 0,5 = plat | `R16Float` |
| Position | Position objet normalisée dans la boîte englobante | `Rgba16Float` |
| Thickness | Rayons tirés vers l'intérieur, distance moyenne | `R16Float` |
| ID | Couleur par matériau, par sous-mesh ou couleurs de vertex du high | `Rgba8Unorm` |
| Height | Distance signée low vers high le long du rayon, normalisée | `R16Float` |

### Paramètres

- Cage (mesh facultatif) ou distances maximales avant et arrière, relatives à la taille du mesh.
- Correspondance des meshes par nom (suffixes `_low` et `_high`) ou tout contre tout.
- Option « normales moyennées » pour la direction des rayons, qui évite les trous aux arêtes dures.
- Supersampling 1x, 4x ou 16x, nombre d'échantillons d'AO (64 à 256), dilatation en sortie.

### Implémentation

1. Rastériser le low poly en espace UV pour obtenir, par texel, le triangle et les coordonnées barycentriques. On en tire la position, la normale et la base tangente interpolées, cohérentes avec mikktspace. Cette étape est commune aux deux versions.
2. Référence CPU : BVH SAH sur le high poly, parallélisation par lignes de texels avec rayon. Le code privilégie la lisibilité.
3. Version GPU : le BVH aplati est envoyé en storage buffer et parcouru dans un compute shader WGSL avec une pile. Le travail est découpé en dispatches par tuiles. Sous Windows, un dispatch trop long déclenche le timeout du pilote (TDR, environ 2 secondes) et fait planter l'application.
4. Le ray tracing matériel via les ray queries de wgpu est encore expérimental. Il vient après le MVP, en option.

Les baked maps sont stockées dans `TextureSet.baked_maps`. Un nouveau bake invalide tous les générateurs qui les lisent.

## Procédural, générateurs, filtres et bibliothèque

Sans Designer, tous les matériaux, générateurs et filtres sont écrits directement en WGSL. Leurs paramètres sont décrits par des données, ce qui permet de générer l'interface automatiquement.

### Description des paramètres

Chaque procédural est déclaré par une `ProceduralDef { id, version, name, category, params: Vec<ParamDef>, shader, outputs }`. Un `ParamDef` est typé : float (min, max, défaut), entier, booléen, couleur, enum, courbe, dégradé ou texture. L'interface du panneau Propriétés se construit à partir de ces définitions, et le layout du uniform buffer du shader aussi. Le champ `version` permet de migrer les paramètres des anciens projets.

### Bibliothèque WGSL (`shaders/procedural/`)

- **Bruits** : value, gradient (Perlin), simplex, Worley (F1, F2, F2 − F1), fBm, ridged, turbulence, domain warp. Tous tileables et pilotés par une graine.
- **Motifs** : dégradés, damier, tuiles, briques, rayures, cercles, rayures aléatoires (scratches), fibres.
- **Projections** : UV, triplanaire avec fondu, planaire, sphérique, cylindrique, avec échelle, rotation et décalage.

### Matériaux procéduraux (au moins 20 au MVP)

Métaux : acier, acier brossé, aluminium, cuivre, laiton, or, fer rouillé. Non-métaux : plastique lisse et grainé, caoutchouc, cuir, tissu, bois, béton, pierre, céramique, verre dépoli, peinture. Chaque matériau produit tous les canaux.

### Générateurs de masques

| Générateur | Entrées | Effet |
| --- | --- | --- |
| Edge Wear | Courbure, bruit | Usure qui découvre les arêtes |
| Dirt | AO, bruit | Crasse dans les creux |
| Dust | World normal (axe Y), bruit | Dépôt sur les faces tournées vers le haut |
| Position Gradient | Position | Dégradé de bas en haut (boue, mousse) |
| Light | World normal, direction | Lumière peinte dans la texture |
| Thickness | Thickness | Zones fines, translucidité |
| Leaks | Position, world normal, bruit étiré | Coulées verticales sous les arêtes |
| Mask Builder | Courbure, AO, position, world normal, bruits | Masque composite entièrement paramétrable |

### Filtres

Flou gaussien, flou directionnel, slope blur, sharpen, HSL, levels, courbes, inversion, postérisation, warp par bruit, clamp, height vers normal, et color to mask (sélection d'une couleur de l'ID map).

### Smart materials et smart masks

- Un smart material est un dossier de calques sérialisé en RON. Il référence les procéduraux par `id` et `version` et contient ses ressources. Un smart mask est une `MaskStack` sérialisée de la même façon.
- Appliquer un preset en fait une copie profonde avec de nouveaux identifiants. Il s'adapte au mesh tout seul, puisque ses générateurs lisent les baked maps.
- Au moins 15 smart materials au MVP : métal usé, peinture écaillée sur métal, plastique sale, bois patiné, cuir usé, etc.

### Bibliothèque d'assets

- Catégories : matériaux, smart materials, smart masks, brosses, alphas, environnements, filtres, générateurs, textures.
- Vignettes rendues par le moteur du viewport (sphère pour les matériaux, plan pour les alphas) et mises en cache sur disque.
- Chaque asset porte ses métadonnées : licence, source, auteur. Le fichier `LICENSES-ASSETS.md` est généré à partir d'elles.
- Import utilisateur par glisser-déposer, avec tags, recherche et favoris.

## Export

L'export transforme les canaux du projet en fichiers prêts pour un moteur. Un preset décrit chaque fichier de sortie et ce que contient chacun de ses canaux RGBA.

### Format d'un preset

```ron
ExportPreset(
    name: "Unreal Engine",
    normal_convention: DirectX,
    outputs: [
        Output(file: "$mesh_$textureSet_BaseColor", format: Png, bit_depth: 8, color_space: Srgb,
               r: Channel(BaseColor, R), g: Channel(BaseColor, G), b: Channel(BaseColor, B), a: None),
        Output(file: "$mesh_$textureSet_ORM", format: Png, bit_depth: 8, color_space: Linear,
               r: Channel(AmbientOcclusion, Gray), g: Channel(Roughness, Gray), b: Channel(Metallic, Gray), a: None),
        Output(file: "$mesh_$textureSet_Normal", format: Png, bit_depth: 16, color_space: Linear,
               r: Normal(X), g: Normal(Y), b: Normal(Z), a: None),
    ],
)
```

Un canal de sortie peut lire un canal du projet, une baked map, une conversion (1 − roughness pour la smoothness, normale et height mélangées, height normalisée) ou une constante.

### Presets livrés

| Preset | Fichiers et packing | Normal map |
| --- | --- | --- |
| Unreal Engine | BaseColor, Normal, ORM (R = AO, G = roughness, B = metallic), Emissive | DirectX (Y−) |
| Unity URP | BaseMap (A = opacité), MetallicSmoothness (R = metallic, A = 1 − roughness), Normal, Occlusion, Emission | OpenGL (Y+) |
| Unity HDRP | BaseMap, MaskMap (R = metallic, G = AO, B = detail mask, A = smoothness), Normal, Emission | OpenGL (Y+) |
| Godot 4 | Albedo, ORM (R = AO, G = roughness, B = metallic), Normal, Emission | OpenGL (Y+) |
| glTF 2.0 | baseColor, metallicRoughness (G = roughness, B = metallic), occlusion (R), normal, emissive, plus un .glb texture | OpenGL (Y+) |
| Canaux séparés | Un fichier par canal du projet | OpenGL (Y+) |

### Options

- Résolution d'export indépendante de la résolution de travail : les strokes sont rejoués.
- Dilatation sur N pixels ou infinie.
- Formats PNG, TGA, TIFF, JPEG et EXR. Profondeur 8 ou 16 bits, ou 32 bits flottants en EXR. Espace couleur par sortie.
- Export de tous les texture sets ou d'une sélection, avec un patron de nom de fichier (`$mesh`, `$textureSet`, `$channel`).

### Export en ligne de commande

`patine_cli export projet.patine --preset unreal --out sortie/ --resolution 4096`. La CI s'en sert pour exporter un projet de test et vérifier le packing pixel par pixel.

## Interface utilisateur

L'interface reprend l'organisation fonctionnelle classique d'un logiciel de texturing : viewport au centre, calques et propriétés à droite, bibliothèque en bas. Le thème sombre et les icônes sont originaux, avec le jeu d'icônes Phosphor (licence MIT) via egui-phosphor.

### Panneaux (egui\_dock, disposition sauvegardée)

| Panneau | Contenu |
| --- | --- |
| Viewport 3D | Rendu, curseur de brosse, gizmos de projection et de symétrie |
| Vue 2D | Texture du texture set, wireframe des UV, peinture en UV |
| Texture Sets | Liste, résolution, canaux actifs, baked maps, bouton Bake |
| Calques | Pile avec vignettes, visibilité, blend mode et opacité du canal sélectionné, masques et effets en sous-éléments, glisser-déposer, menu contextuel |
| Propriétés | Outil actif, ou calque, masque ou effet sélectionné, généré depuis les `ParamDef` |
| Bibliothèque | Catégories, recherche, vignettes, glisser-déposer vers les calques ou le viewport |
| Affichage | HDRI, rotation, exposition, tone mapping, mode d'affichage |
| Historique | Liste des commandes, clic pour revenir à un état |
| Console | Logs, erreurs, activation du profileur puffin |

Dialogues : nouveau projet (mesh, modèle de canaux, résolution, convention de normal map), bake, export, préférences.

### Raccourcis par défaut (tous reconfigurables)

| Action | Raccourci |
| --- | --- |
| Orbite, panoramique, zoom | Alt + clic gauche, milieu, droit ; molette |
| Tourner l'environnement | Maj + clic droit, glisser horizontal |
| Cadrer le mesh | F |
| Brosse, gomme, projection, polygon fill | B, E, P, G |
| Clone, smudge, pipette | K, S, I |
| Taille de brosse, dureté | \[ et \], Maj + \[ et \] |
| Ligne droite | Maj + clic |
| Symétrie | L |
| Matériau éclairé ou canal isolé | F1, puis C pour faire défiler les canaux |
| Annuler, rétablir, enregistrer | Ctrl + Z, Ctrl + Y, Ctrl + S |
| Palette de commandes | Ctrl + K |

### Règles d'implémentation

- Toutes les actions sont déclarées dans un registre unique (`Action` : id, libellé, raccourci, condition d'activation). Les menus, la palette de commandes et les raccourcis lisent ce registre.
- L'interface ne garde aucun état qui appartient au document. Elle lit le document et émet des commandes.
- Tous les textes passent par des fichiers de traduction (crate fluent) dès le M0, en français et en anglais.

## Tests et budgets de performance

Chaque algorithme GPU a une référence CPU et un test de comparaison. Chaque milestone a des budgets de performance mesurés sur la RTX 3080 Ti et sur la RTX A2000.

### Niveaux de tests

1. **Unitaires** (`patine_core`, sans GPU) : `apply` et `revert` de chaque commande, sérialisation aller-retour, migrations, graphe d'évaluation (tri, refus des cycles).
2. **Propriétés** (proptest) : une séquence aléatoire de commandes, suivie d'une annulation complète, redonne le document initial. Sauvegarder puis recharger redonne un document identique.
3. **GPU contre référence CPU** : blend modes, bruits, filtres, baker. Une macro `gpu_test!` crée un device wgpu headless. En CI, l'adaptateur est logiciel : WARP sous Windows, lavapipe sous Linux.
4. **Images de référence** : scènes du viewport rendues et comparées à `tests/golden/` avec un seuil d'erreur. Les références sont générées sur l'adaptateur logiciel de la CI, et la tolérance est plus large sur un vrai GPU. `PATINE_UPDATE_GOLDEN=1` les régénère.
5. **Shaders** : un test parse et valide tous les fichiers WGSL avec naga.
6. **Bout en bout** : `patine_cli` importe un mesh, bake, applique un smart material, exporte, puis le test vérifie les fichiers et quelques pixels.
7. **Tests manuels** : chaque note `docs/design/Mxx.md` se termine par une checklist de ce que l'humain doit essayer à la main.

Les meshes de test sont générés par le code (cube, sphère, cylindre avec coutures UV, paire low/high par subdivision et déplacement). Aucun problème de licence, et les résultats attendus sont connus.

### Budgets de performance

Ce sont des objectifs de départ, à ajuster après les premières mesures. `patine_cli bench` les mesure de façon reproductible.

| Mesure | RTX 3080 Ti | RTX A2000 portable |
| --- | --- | --- |
| Viewport, mesh de 500 000 triangles, textures 4K | ≥ 120 i/s | ≥ 60 i/s |
| Latence d'un dab (4K, 20 calques) | < 8 ms | < 16 ms |
| Paramètre d'un fill en bas d'une pile de 30 calques 4K | < 100 ms | < 250 ms |
| Bake normal + AO (128 échantillons), 4K, high poly de 2 M triangles | < 30 s | < 90 s |
| Ouverture d'un projet 4K de 30 calques | < 5 s | < 10 s |
| Export 4K, preset Unreal, un texture set | < 5 s | < 10 s |
| Mémoire GPU, projet 4K de 30 calques | < 6 Go | < 3,5 Go |

## Milestones du MVP

Le MVP compte 18 milestones, à réaliser dans l'ordre. Seule exception : le baker (M11 et M12) ne dépend que de l'import (M1) et peut avancer en parallèle des milestones M5 à M10. Claude Code coche les tâches au fur et à mesure, et un milestone n'est terminé que si tous ses critères d'acceptation sont vérifiés.

### M0 — Fondations

Objectif : une application qui s'ouvre, une CI verte et la structure du dépôt en place.

- [x] Workspace Cargo avec toutes les crates, vides, et leurs dépendances autorisées
- [x] Fenêtre winit + wgpu + egui, disposition egui\_dock avec les panneaux vides
- [x] Logs tracing, profileur puffin activable, panneau Console
- [x] Registre d'actions et fichiers de traduction FR et EN
- [x] Macro `gpu_test!`, test de validation des shaders, comparateur d'images de référence
- [ ] CI GitHub Actions : fmt, clippy `-D warnings`, tests sur WARP et lavapipe, cargo-deny

Critères d'acceptation : l'application démarre sous Windows ; la CI est verte sous Windows et Linux ; un test GPU trivial (remplir une texture et la relire) passe.

### M1 — Import de meshes

Objectif : charger un mesh et en extraire tout ce dont les autres sous-systèmes ont besoin.

- [x] Import glTF, OBJ et FBX vers `MeshAsset` : positions, normales, UV0, couleurs de vertex, sous-meshes par matériau
- [x] Triangulation, normales manquantes, tangentes mikktspace
- [x] Adjacence : polygones d'origine, îlots UV, parties connexes
- [x] BVH SAH aplati, réutilisé par le picking, la peinture et le baker
- [x] Générateurs de meshes de test : cube, sphère, cylindre, paire low/high
- [x] Dialogue Nouveau projet, avec un texture set par matériau

Critères d'acceptation : les trois formats chargent les fixtures ; le nombre d'îlots UV est exact sur les meshes générés ; un mesh sans UV est refusé avec un message clair.

### M2 — Viewport PBR

Objectif : un viewport dont le rendu correspond à celui des moteurs cibles.

- [x] Caméra : orbite, panoramique, zoom, cadrage, perspective ou orthographique
- [x] Shading GGX metal/roughness avec un matériau constant
- [x] HDRI : cubemap, irradiance SH, préfiltrage spéculaire, LUT de BRDF
- [x] Tone mapping AgX, ACES et PBR Neutral ; exposition ; rotation de l'environnement
- [x] Shadow map directionnelle, MSAA 4x, wireframe
- [x] Buffer de picking (triangle + profondeur)

Critères d'acceptation : l'image de référence d'une grille de sphères roughness × metallic passe ; le budget du viewport est tenu ; le picking renvoie le bon triangle sur les meshes de test.

### M3 — Document, commandes et sauvegarde

Objectif : un modèle de document fiable, entièrement annulable et sauvegardable.

- [ ] Types du modèle de données (section Architecture)
- [ ] `Command` avec `apply` et `revert`, fusion des commandes continues, historique borné en mémoire
- [ ] Format `.patine` : zip, RON, postcard + zstd, `format_version`, migrations
- [ ] Autosave et récupération après crash
- [ ] Panneau Historique

Critères d'acceptation : les tests de propriétés sur l'undo et la sauvegarde passent ; un projet sauvegardé se rouvre à l'identique.

### M4 — Texture sets et canaux sur GPU

Objectif : afficher les canaux d'un texture set et gérer la mémoire GPU.

- [ ] `GpuDocument`, pool de textures avec comptage mémoire et éviction LRU
- [ ] Panneau Texture Sets : résolution, canaux actifs, valeurs par défaut
- [ ] Affichage des canaux dans le viewport, modes canal isolé, vue 2D avec wireframe UV

Critères d'acceptation : changer de résolution réalloue sans fuite, et la mémoire mesurée revient à la valeur attendue ; la vue 2D affiche le bon wireframe UV.

### M5 — Compositeur v1

Objectif : une pile de calques fill complète et rapide.

- [ ] Calques fill (valeur uniforme par canal), dossiers, visibilité, ordre
- [ ] Blend modes et opacité par canal, Normal Map Combine
- [ ] Graphe d'évaluation, invalidation, cache inférieur, tuiles sales
- [ ] Référence CPU de tous les blend modes
- [ ] Panneau Calques : création, suppression, glisser-déposer, renommage

Critères d'acceptation : chaque blend mode correspond à la référence CPU à 1/512 près ; le budget « paramètre d'un fill » est tenu.

### M6 — Masques

Objectif : masquer n'importe quel calque avec une pile d'effets.

- [ ] `MaskStack` : fill, levels, inversion, blend mode et opacité par effet
- [ ] Masques noir ou blanc, ajout par menu contextuel
- [ ] Mode d'affichage du masque sélectionné

Critères d'acceptation : les masques composés correspondent à la référence CPU ; chaque opération s'annule.

### M7 — Bibliothèque procédurale

Objectif : des fills procéduraux paramétrables, avec une interface générée.

- [ ] `ProceduralDef`, `ParamDef`, génération automatique du panneau Propriétés
- [ ] Bruits, motifs et projections en WGSL, tileables et pilotés par graine
- [ ] Fills avec matériau procédural, avec les 5 premiers matériaux

Critères d'acceptation : chaque bruit est tileable (bords identiques à 1e-3 près) ; la projection triplanaire ne montre pas de couture sur la sphère de test ; un paramètre modifié se voit en moins de 100 ms.

### M8 — Peinture v1

Objectif : peindre un matériau directement sur le mesh.

- [ ] Échantillonneur de stroke (espacement, lissage) et `StrokeLog`
- [ ] `rasterize_dabs` : projection écran vers UV, occlusion, atténuation selon l'angle, buffer de stroke
- [ ] Brosse et gomme, peinture multi-canaux, alphas
- [ ] Undo par tuiles, dilatation à la composition
- [ ] Curseur de brosse dans le viewport

Critères d'acceptation : un trait qui traverse une couture UV ne laisse aucun trou sur le cube de test ; rejouer le `StrokeLog` reproduit l'état peint en direct à la tolérance près ; le budget de latence est tenu.

### M9 — Peinture v2

Objectif : le confort de peinture d'un outil professionnel.

- [ ] Pression et inclinaison du stylet (octotablet), courbes de pression
- [ ] Jitters, scatter, suivi de trajectoire, presets de brosses
- [ ] Symétrie miroir et radiale
- [ ] Ligne droite, pipette, peinture dans la vue 2D
- [ ] Canal height peint et converti en normale en temps réel

Critères d'acceptation : une tablette Wacom fonctionne sous Windows (test manuel) ; la symétrie est exacte sur la sphère ; le rejeu avec jitter est déterministe.

### M10 — Indépendance de résolution

Objectif : changer de résolution sans perte de qualité.

- [ ] Rejeu complet des strokes à une nouvelle résolution
- [ ] Résolution d'export distincte de la résolution de travail
- [ ] Caches raster facultatifs dans le fichier projet

Critères d'acceptation : peindre en 1K, passer en 4K puis réduire en 1K redonne l'original à la tolérance près ; le résultat en 4K est net, pas agrandi.

### M11 — Baker CPU de référence

Objectif : un baker correct, même lent, qui sert de référence.

- [ ] Rastérisation UV du low poly : triangle et barycentriques par texel
- [ ] Les 8 maps, cage ou distances, normales moyennées, supersampling
- [ ] Correspondance par nom, dialogue de bake, stockage dans `baked_maps`

Critères d'acceptation : pour une sphère fine bakée sur une sphère grossière, la normal map correspond au résultat analytique ; l'AO d'un plan isolé vaut 1 ; une normal map bakée s'affiche dans le viewport sans couture visible.

### M12 — Baker GPU

Objectif : le même résultat que le baker CPU, des dizaines de fois plus vite.

- [ ] BVH en storage buffer, parcours en WGSL, dispatches par tuiles
- [ ] Choix CPU ou GPU dans le dialogue, barre de progression, annulation

Critères d'acceptation : la sortie GPU correspond à la sortie CPU à la tolérance près ; le budget de bake est tenu ; aucun timeout du pilote sur la RTX A2000.

### M13 — Générateurs et smart masks

Objectif : des masques qui s'adaptent automatiquement à la forme du mesh.

- [ ] Les 8 générateurs, branchés sur les baked maps
- [ ] Invalidation des générateurs après un nouveau bake
- [ ] Format smart mask, application, sauvegarde depuis un masque existant

Critères d'acceptation : Edge Wear suit les arêtes du mesh de test (image de référence) ; un nouveau bake met les masques à jour sans action de l'utilisateur.

### M14 — Polygon fill et ID map

Objectif : délimiter des zones par géométrie ou par couleur d'ID.

- [ ] Sélection par triangle, face, îlot UV, partie de mesh ou couleur d'ID
- [ ] Rastérisation de la sélection en masque, stockée comme `PolygonFill`
- [ ] Filtre color to mask

Critères d'acceptation : remplir un îlot couvre exactement ses texels sur le cube de test ; la sélection survit à un changement de résolution.

### M15 — Filtres, anchor points et instances

Objectif : les fonctions avancées de la pile de calques.

- [ ] Tous les filtres de la liste, comme effets de calque et de masque
- [ ] Anchor points : capture, référence, invalidation, refus des cycles
- [ ] Calques instance entre texture sets

Critères d'acceptation : une modification sous un anchor met à jour le masque qui le lit ; chaque filtre correspond à sa référence CPU.

### M16 — Smart materials et bibliothèque

Objectif : un contenu de départ suffisant pour texturer un asset sans rien créer.

- [ ] Format smart material, création depuis un dossier, application
- [ ] 20 matériaux procéduraux et 15 smart materials
- [ ] Panneau Bibliothèque : catégories, recherche, vignettes en cache, import utilisateur
- [ ] Métadonnées de licence et `LICENSES-ASSETS.md`

Critères d'acceptation : un smart material appliqué sur un nouveau mesh baké donne un rendu cohérent sans réglage ; une vignette en cache s'affiche en moins de 200 ms.

### M17 — Export et finition du MVP

Objectif : livrer des textures utilisables dans les moteurs, et un MVP stable.

- [ ] Moteur d'export, format de preset, les 6 presets livrés
- [ ] Dilatation, formats, profondeurs, patrons de noms
- [ ] `patine_cli export` et test de bout en bout
- [ ] Préférences, éditeur de raccourcis, messages d'erreur clairs pour l'utilisateur
- [ ] Passe de performance sur tous les budgets

Critères d'acceptation : un export glTF s'ouvre dans Blender avec le bon rendu (test manuel) ; le packing est vérifié pixel par pixel en CI ; tous les budgets sont tenus, ou l'écart est documenté.

## Après le MVP

Ces évolutions ne commencent qu'une fois le MVP stable. Elles sont classées par ordre de priorité suggéré : les deux premières libèrent des utilisateurs professionnels, les suivantes différencient le produit.

1. **UDIM et textures virtuelles** : plusieurs tuiles UV par texture set, et pagination des tuiles pour travailler en 8K.
2. **macOS et Linux** : packaging, signature, tests sur Metal.
3. **API de plugins Python** (pyo3) : accès au document, à l'export et aux commandes.
4. **Live link** vers Blender, Unity, Unreal et Godot : les textures sont renvoyées au moteur à chaque modification.
5. **Fonctions IA locales** : texte vers matériau, photo vers matériau, upscaling. Les modèles tournent sur le GPU de l'utilisateur ; leur licence doit autoriser un usage commercial.
6. **Rendu path tracé** de présentation, sur les ray queries de wgpu une fois stabilisées.
7. **Brosses à particules physiques** : pluie, coulées, éclaboussures.
8. **Formats** : import et export MaterialX et USD.
9. **Rendu** : parallax occlusion mapping, TAA, aperçu de subsurface.
10. **Version web** d'une partie de l'outil sur WebGPU, puisque wgpu compile vers WebAssembly.

## Contenu prêt à copier dans CLAUDE.md

Ce fichier se place à la racine du dépôt. Claude Code le lit au début de chaque session : il contient les commandes, la méthode de travail et les règles à ne jamais enfreindre.

```markdown
# Patine — instructions pour Claude Code

Patine est un logiciel de texturing PBR par calques sur mesh 3D (équivalent fonctionnel de Substance Painter).
Le plan complet est dans `docs/PLAN.md` : c'est la source de vérité pour la stack, l'architecture et les milestones.

## Commandes
- Lancer : `cargo run -p patine_app`
- Lancer avec rechargement des shaders : `cargo run -p patine_app --features hot-reload`
- Tests : `cargo test --workspace`
- Lint : `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
- Licences : `cargo deny check`
- Régénérer les images de référence : `PATINE_UPDATE_GOLDEN=1 cargo test -p <crate>`

## Méthode de travail
1. Avant un milestone, relis sa section dans `docs/PLAN.md` et les sections d'architecture qu'il touche.
2. Commence en plan mode. Écris `docs/design/Mxx-<nom>.md` : choix de conception, structures, risques, checklist de tests manuels.
3. Avance par petites tâches. Après chaque tâche : fmt, clippy, tests, puis un commit (Conventional Commits).
4. Coche la tâche dans `docs/PLAN.md` seulement quand ses tests passent.
5. Un milestone n'est terminé que si tous ses critères d'acceptation sont vérifiés. Termine par un résumé : fait, reste à faire, tests manuels à réaliser.
6. Après trois échecs sur le même problème, arrête-toi et écris un rapport (hypothèses testées, pistes) au lieu de contourner.

## Règles d'architecture
- `patine_core` ne dépend jamais de wgpu ni d'egui.
- Toute modification du document passe par une `Command` avec `apply` et `revert`. L'UI n'écrit jamais dans le document.
- Le GPU n'est qu'un cache : aucune donnée du document n'existe seulement en pixels, sauf les images importées.
- Une seule fonction rastérise les strokes (`rasterize_dabs`), pour la peinture en direct comme pour le rejeu.
- Chaque algorithme GPU a une implémentation CPU de référence et un test de comparaison.
- Shaders en WGSL dans `shaders/`, validés par naga en test. Aucun shader inline dans le code Rust.
- Les dispatches GPU longs sont découpés par tuiles (timeout du pilote Windows d'environ 2 s).
- Les actions de l'UI passent par le registre `Action`. Tous les textes passent par les fichiers de traduction.

## Code
- Rust stable, édition 2024. Code, identifiants et commentaires en anglais ; notes de conception en français.
- `thiserror` dans les crates, `anyhow` seulement dans les binaires.
- Pas de `unwrap()` hors des tests. `expect()` avec un message qui explique l'invariant.
- Pas de `unsafe` sans commentaire `// SAFETY:` qui le justifie.
- Aucune nouvelle dépendance sans justification dans le message de commit. Licences permissives uniquement.
- Fonctions publiques documentées. Mesurer avant d'optimiser (puffin ou `patine_cli bench`).

## Interdits
- Jamais les mots « Substance » ou « Adobe » dans le produit, le code ou la doc utilisateur.
- Jamais d'assets, d'icônes ou de formats de fichiers Adobe (.spp, .sbsar, .sbs, .sppr).
- Jamais d'asset sans licence claire : CC0 ou généré, avec métadonnées de licence.
- Ne jamais désactiver ou affaiblir un test pour le faire passer. Si un test est faux, explique pourquoi avant de le modifier.

## Machine de développement
- Windows avec RTX 3080 Ti (référence haute) et portable RTX A2000 (référence basse).
- Les budgets de performance sont dans `docs/PLAN.md`.
```

## Prompts pour Claude Code

Le premier prompt lance le projet. Le second sert de modèle pour chaque milestone suivant, avec tes retours de tests manuels.

### Première session

```text
Lis CLAUDE.md, puis docs/PLAN.md en entier.
Nous démarrons le milestone M0 — Fondations.

1. Passe en plan mode et propose ton découpage de M0 en tâches. Vérifie sur crates.io la dernière version stable de chaque crate de la stack.
2. Avant de coder, signale toute incohérence ou tout risque que tu vois dans le plan.
3. Après ma validation, implémente M0 tâche par tâche en suivant la méthode de CLAUDE.md.
4. Termine par le résumé de fin de milestone et la checklist des tests manuels.
```

### Sessions suivantes

```text
Lis CLAUDE.md, la section Mxx de docs/PLAN.md, les sections d'architecture qu'elle touche, et les notes docs/design des milestones précédents.
Nous démarrons le milestone Mxx — <nom>.

Retours de mes tests manuels du milestone précédent :
- <à compléter>

Traite d'abord ces retours. Ensuite, passe en plan mode, écris docs/design/Mxx-<nom>.md, attends ma validation, puis implémente.
```
