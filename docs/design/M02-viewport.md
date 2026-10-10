# M2 — Viewport PBR

Note de conception du milestone M2. Objectif : un viewport dont le rendu correspond à celui des moteurs cibles.

Critères d'acceptation :
1. l'image de référence d'une grille de sphères roughness × metallic passe ;
2. le budget du viewport est tenu (500 k triangles : ≥ 120 i/s sur la RTX 3080 Ti, ≥ 60 sur la RTX A2000) ;
3. le picking renvoie le bon triangle sur les meshes de test.

## Architecture

Le viewport rend hors écran puis s'affiche dans egui comme une image :

1. passe d'ombre (shadow map de la lumière extraite de l'HDRI) ;
2. passe PBR en MSAA 4x, couleur `Rgba16Float` linéaire, profondeur reverse-Z `Depth32Float` ;
3. fond (environnement flouté ou couleur unie) dans la même passe, derrière le mesh ;
4. wireframe des polygones d'origine (`LineList`), dans la même passe ;
5. résolution du MSAA ;
6. tone mapping et exposition vers une texture `Rgba8Unorm` encodée en sRGB, le format qu'attend `egui_wgpu::Renderer::register_native_texture`.

Le rendu 3D est encodé avant la passe egui de la même frame, à la taille du panneau mesurée à la frame précédente : un redimensionnement a une frame de retard, ce qui est invisible en pratique.

Le même `Viewport` rend sans fenêtre pour les tests et pour `patine_cli bench viewport`.

## Caméra

`OrbitCamera` : cible, distance, lacet, tangage, FOV vertical, perspective ou orthographique. Profondeur reverse-Z (proche = 1, loin = 0), pour une meilleure précision sur les grandes scènes. Le cadrage (F) ajuste la distance pour que la sphère englobante tienne dans le champ.

Les contrôles (Alt + clic gauche, milieu, droit ; molette ; Maj + clic droit pour l'environnement) sont des fonctions pures de la caméra ; l'app ne fait que leur passer les déplacements de la souris.

## Environnement

- Sources :
  - ciel de studio procédural généré par le code, déterministe, utilisé par défaut et par les tests ;
  - HDRI `.hdr` ou `.exr` (crate `image`).
- Les pixels de l'HDRI sont envoyés en `Rgba32Float` et lus par `textureLoad` : la plage de f16 (65 504) ne suffit pas pour le soleil d'une HDRI.
- Traitements GPU (compute), chacun avec sa référence CPU dans `patine_render::reference` :
  - équirect → cubemap 512² par face, `Rgba16Float`, avec mips ;
  - irradiance en harmoniques sphériques d'ordre 2 (9 coefficients RGB) ;
  - cube spéculaire préfiltré GGX, 256², 6 mips de roughness 0 à 1, échantillonnage d'importance filtré par mip ;
  - LUT BRDF 2D `Rg16Float` 256² (échelle et biais de F0).
- Les dispatches sont découpés par face, par mip et par tuile : le pilote Windows coupe les dispatches de plus de 2 s.

Les réglages d'environnement (HDRI, rotation, exposition) appartiennent au document selon le plan. Les commandes n'arrivant qu'au M3, ils vivent pour l'instant dans l'état de vue de l'app et passeront dans `Project` au M3.

## Shading

- Cook-Torrance : distribution GGX, visibilité Smith height-correlated, Fresnel de Schlick, F0 = 0,04 pour les diélectriques.
- Diffus Lambert pondéré par (1 − F)(1 − metallic).
- IBL :
  - diffus par les SH ;
  - spéculaire par le cube préfiltré × (F0 · A + B) de la LUT.
- L'AO n'atténue que l'éclairage indirect.
- Matériau constant au M2 (couleur de base, metallic, roughness), réglable dans le panneau Paramètres d'affichage. Les normal maps et les textures arrivent au M4.

### Ombres

La lumière directionnelle est extraite des SH de l'environnement (méthode de Sloan, *Stupid SH Tricks*) :
- direction = bande L1 de la luminance ;
- intensité = projection aux moindres carrés d'une lumière directionnelle sur les 9 coefficients.

Elle est retirée de l'irradiance diffuse, puis rajoutée avec N·L et le facteur d'ombre (shadow map 2048², ortho ajustée au mesh, PCF). Sans occlusion, le résultat est identique à l'IBL pure, à l'approximation SH près. Quand les ombres sont désactivées, on utilise directement les SH complètes.

Limite assumée : le reflet spéculaire de l'environnement n'est pas ombré.

## Tone mapping

Exposition en EV appliquée avant l'opérateur. Opérateurs :
- **AgX** (défaut) : approximation polynomiale de référence, look de base ;
- **ACES** : fit de Hill (RRT + ODT) ;
- **Khronos PBR Neutral** : formule de la spécification Khronos ;
- **linéaire** : écrêtage.

Encodage sRGB final dans le shader. Chaque opérateur a une référence CPU testée sur une rampe HDR.

## Wireframe

Seules les arêtes des polygones d'origine sont tracées : une arête est gardée si ses deux triangles appartiennent à des polygones différents, ou si c'est un bord. Les diagonales de triangulation restent invisibles.

## Picking

Une passe rend l'identifiant de triangle (`R32Uint`, 0 = fond) et la profondeur ; on lit le pixel sous le curseur de façon asynchrone, et le résultat arrive à la frame suivante.
- Avec `Features::PRIMITIVE_INDEX` : `@builtin(primitive_index)` sur le mesh indexé.
- Sinon : buffers dé-indexés avec un identifiant plat, réservés à cette passe.

Le test compare le triangle GPU à `Bvh::closest_hit` (M1) pour le rayon caméra de chaque pixel testé. Il passe sur les deux chemins.

## Shaders partagés

`patine_gpu::shaders` gagne un préprocesseur : une ligne `// #import chemin` est remplacée par le fichier `shaders/chemin.wgsl`, inclus une seule fois par shader composé ; les cycles sont refusés. Le test naga valide les shaders composés.

## Budget

`patine_cli bench viewport` rend hors écran en 1920 × 1080, MSAA 4x, ombres et IBL actifs, une sphère de 500 k triangles, et affiche les images par seconde. Les textures 4K n'existent pas encore : la mesure sera refaite au M4.

## Risques

- Les traitements d'environnement sont lents sur WARP : les tests utilisent de petites résolutions.
- L'image de référence est générée sur WARP ; un vrai GPU utilise la tolérance « matériel ».
- Alt + clic sous Windows : Alt seul peut activer le menu système de la fenêtre.

## Checklist de tests manuels

- [ ] Ouvrir chaque fixture du M1 : le mesh apparaît cadré, éclairé, sans facettes inattendues.
- [ ] Alt + clic gauche, milieu, droit et molette : orbite, panoramique, zoom fluides ; F recadre.
- [ ] Projection orthographique et FOV.
- [ ] Changer d'environnement, de rotation (Maj + clic droit), d'exposition et d'opérateur : le rendu suit sans saccade.
- [ ] Ombres : le cube posé sur lui-même projette une ombre cohérente avec la direction du soleil de l'HDRI.
- [ ] Wireframe : les quads apparaissent sans diagonale.
- [ ] Survol : la barre d'état affiche le triangle sous le curseur.
- [ ] Redimensionner et détacher le panneau Vue 3D.
- [ ] `cargo run --release -p patine_cli -- bench viewport` sur la RTX A2000 : au moins 60 i/s.
