# M9 — Peinture v2

Note de conception du milestone M9. Objectif : le confort de peinture d'un outil professionnel.

Critères d'acceptation :
1. une tablette Wacom fonctionne sous Windows (test manuel) ;
2. la symétrie est exacte sur la sphère ;
3. le rejeu avec jitter est déterministe.

Le M8 a reporté au M9 la peinture d'un matériau procédural (PLAN, « Paramètres de brosse ») : elle est incluse ici.

## Décisions

1. **Format v2** (`FORMAT_VERSION` = 2).
   - `BrushSnapshot` gagne :
     - `pressure: PressureCurves { size, flow, opacity }` : courbes linéaires par morceaux, identité par défaut ;
     - `tilt_angle: bool` : l'inclinaison du stylet oriente les dabs ;
     - `projection: Projection` : la projection d'un matériau procédural, triplanaire par défaut.
   - `Stroke` gagne `space: StrokeSpace { View, Uv }`, pour les traits peints dans la vue 2D.
   - `TextureSet` gagne `height_force: f32` (1 par défaut), modifiée par la commande `SetHeightForce`, fusionnable.
   - **Types v1 figés** : ils vont dans `file::v1` avec leur migration (courbes identité, pas d'inclinaison, triplanaire, `View`, force 1). Un trait v1 se rejoue donc à l'identique.
   - **Strokes en postcard** : ce format ne décrit pas ses champs, donc les strokes v1 sont lus avec les types v1, puis migrés.
   - **Fixtures** : `v1_rich` reste relue, et `v2_rich` est ajoutée.
2. **Dynamique des dabs**, déterministe.
   - `gpu_dabs(stroke, range)` remplace `gpu_dab`. Le dab `i` ne dépend que de la graine, de `i` et des dabs `0..=i`, donc le découpage en lots ne change rien.
   - **Pression** : des courbes la convertissent en taille, en flux et en opacité.
   - **Inclinaison** : elle donne l'angle si `tilt_angle`.
   - **Suivi de trajectoire** : l'angle suit la direction depuis le dab précédent.
   - **Jitters et scatter** : leurs valeurs viennent d'un hash entier de `(graine, i)`, identique en Rust et en WGSL. Le scatter décale le dab perpendiculairement à la trajectoire.
   - **Opacité par dab** : l'accumulation est plafonnée dab par dab, `s' = max(s, min(o_i, s + (1 − s)·a_i))`. Avec `o_i = 1`, c'est exactement la formule du M8.
   - **Graine** : l'application en tire une par trait.
3. **Symétrie : une caméra miroir par copie.**
   - **Copies** : l'identité, les miroirs X, Y et Z autour de l'origine de l'objet, et les rotations radiales autour de Y. Chaque copie `T` a sa caméra `view_proj × T` et son image de profondeur, dans une texture-array.
   - **Accumulation** : dans le shader, les dabs sont en boucle externe et les copies en boucle interne.
   - **Tuiles touchées** : la requête BVH transforme les coins des boîtes par `T` avant de les projeter.
   - **Sur un mesh symétrique**, c'est exactement la peinture au point miroir.
   - **Plafond** : 16 copies.
4. **Peinture en UV** (vue 2D), avec `StrokeSpace::Uv`.
   - **Caméra** : elle envoie le point `(u, v, 0, 1)` sur les pixels de la vue 2D (`UvTransform`).
   - **Shader** : il projette l'UV du point de surface, sans test de profondeur ni atténuation d'angle.
   - **Tuiles touchées** : celles du rectangle des dabs en UV, élargi de la marge de dilatation.
   - **Symétrie** : aucune en UV.
5. **Matériau procédural peint** : `material = FillSource::Procedural` avec sa `projection`.
   - **Évaluation** : par le chemin du M7, à la résolution du set.
   - **Cache** : dans `PaintRasters`, par (procédural, projection, taille), jetable.
   - **Composition** : elle lit la valeur dans ces textures au lieu d'une constante.
6. **Hauteur convertie en normale, en temps réel.**
   - **Normale d'ombrage** : une par texture set (`Rgba16Float` avec mips). C'est la combinaison RNM du canal normal (ou de `(0, 0, 1)`) et de la normale de hauteur.
   - **Normale de hauteur** : par différences centrées, `n = normalize(−k·∂h/∂u, −k·∂h/∂v, 1)` avec `k = height_force × HEIGHT_SCALE`, les gradients étant en unités UV.
   - **Mise à jour** : sur la région, à chaque changement de la hauteur ou de la normale, pendant un trait aussi.
   - **Affichage** : le viewport la lie au slot Normal. Les modes canal isolé montrent toujours les canaux bruts.
7. **Tablette** (`patine_app::tablet`, octotablet 0.1, Windows Ink).
   - **Gestionnaire** : il est construit sur la fenêtre et pompé à chaque image.
   - **Trait au stylet** : un stylet en contact donne les points du trait, avec la pression (normalisée par les limites de l'axe) et l'inclinaison. Les événements souris émulés sont ignorés pendant que le stylet est en proximité.
   - **Repli** : si octotablet échoue, la souris reste disponible (pression 1), et un avertissement s'affiche dans la console.
8. **Presets de brosse** (`BrushPreset`, RON) : un nom et les réglages de la brosse, matériau compris.
   - **Presets livrés** (générés par code) : Standard, Aérographe, Feutre dur, Craie, Éclaboussures, Rayures.
   - **Presets de l'utilisateur** : enregistrés dans `presets/`, dans le dossier de données de l'application.
9. **Outils de l'application.**
   - **Ligne droite** : Maj + clic, depuis la fin du trait précédent, dans la même vue.
   - **Pipette** (I) : elle lit la valeur de chaque canal sous le curseur (la valeur du masque pour un masque), puis revient à l'outil précédent.
   - **Symétrie** : L active ou coupe le miroir X.
   - **Vue 2D** : on y peint par clic gauche glissé.
   - **Réglages** : jitters et éditeur de courbes de pression ; choix d'un matériau procédural ; force du relief dans le panneau Texture sets.

## Structures

- **`patine_core`** :
  - `PressureCurves`, `StrokeSpace`, `TextureSet.height_force`, `SetHeightForce` ;
  - `file::v1`, avec la migration des manifestes et des strokes.
- **`patine_paint`** :
  - `dynamics.rs` : `gpu_dabs`, courbes, hash ;
  - `symmetry.rs` : copies `T` ;
  - `projection.rs` : tuiles par copie et en UV ;
  - `raster.rs` : texture-array de profondeur, tampon de caméras, valeurs en texture ;
  - `reference.rs`.
- **`patine_compositor`** :
  - cache des matériaux de brosse dans `PaintRasters` ;
  - normale d'ombrage dans `GpuDocument` ;
  - shader `height_normal.wgsl`.
- **Application** :
  - `tablet.rs` ;
  - `presets.rs` ;
  - `paint_tool.rs` : dynamique, courbes, symétrie, pipette, matériau ;
  - peinture dans `view_2d.rs` ;
  - actions I et L.

## Tests

- **Format** : migration v1 → v2 (fixture `v1_rich` relue), fixture `v2_rich`, aller-retour et proptests.
- **Dynamique** :
  - courbes de pression ;
  - dabs identiques quel que soit le découpage en lots (proptest) ;
  - une graine différente change les dabs.
- **Critère 3** :
  - un trait avec jitters, scatter, suivi, pression et symétrie, peint en direct par lots, égale son rejeu à l'octet près ;
  - deux rejeux sont identiques.
- **Critère 2** :
  - trait avec miroir X sur la sphère : la couverture en P égale celle au point miroir M(P), lue par les UV ;
  - un témoin sans symétrie échoue ;
  - même vérification avec un radial.
- **Références** : GPU = CPU pour les copies, le mode UV, le plafond d'opacité, le matériau procédural et la normale d'ombrage.
- **Tablette** : fonctions pures (pression, coordonnées, choix entre stylet et souris).

## Risques

- **octotablet 0.1** : jeune, et testé avec winit 0.29 (l'app est en 0.30). On le valide tôt ; en cas d'échec, repli sur la souris et rapport.
- **Coût de la symétrie** : jusqu'à 16 copies par texel et par dab, plus 16 images de profondeur au début du trait. Le bench `--symmetry` le mesure.
- **Migration des strokes postcard** : couverte par les fixtures.

## Checklist de tests manuels

1. Wacom sous Windows : la pression module la taille, le flux et l'opacité selon les courbes ; l'inclinaison oriente un alpha allongé.
2. Presets : chacun donne son trait ; enregistrer un preset, relancer l'application, le retrouver ; le supprimer.
3. Jitters, scatter et suivi de trajectoire : le trait varie, et Ctrl+Z puis Ctrl+Y redonne le même trait.
4. Symétrie X sur le casque, et radiale 6 sur une sphère : les copies apparaissent en direct, au bon endroit.
5. Maj + clic : une ligne droite depuis la fin du trait précédent.
6. Pipette (I) sur une zone peinte : la brosse prend ses valeurs.
7. Vue 2D : peindre directement sur les UV ; le trait apparaît aussi dans la vue 3D.
8. Matériau procédural (cuivre, bois) peint au pinceau.
9. Peindre le canal height : le relief apparaît en direct dans la vue 3D ; « Force du relief » le règle.
10. Enregistrer puis rouvrir : identique. Un projet M8 (format v1) s'ouvre, et ses traits sont identiques.
11. `patine_cli bench paint --symmetry 8` et `--symmetry 8 --mirror` sur la RTX A2000 : moins de 16 ms par dab.
12. Vue 2D au stylet : la pression module le trait, et le stylet ne peint pas deux fois (souris émulée ignorée).

## Mesures (14/10/2026, RTX 3080 Ti, build release)

`cargo run --release -p patine_cli -- bench paint [--symmetry N] [--mirror] [--preset NAME] [--size PX]`. La scène est celle du M8 :
- un calque de peinture au milieu de 20 calques, sur une sphère, en 4096² avec 5 canaux ;
- le trait suit un cercle de 200 dabs, à raison d'un dab par image ;
- chaque dab est chronométré jusqu'à la fin du travail du GPU ;
- la ligne `cpu` du bench donne le temps de préparation côté CPU.

| Brosse | Copies | Moyenne | 95e centile | Pire dab | CPU |
| --- | --- | --- | --- | --- | --- |
| 80 px | 1 | 2,4 ms | 3,6 ms | 4,0 ms | 1,0 ms |
| 80 px | 2 (radial 2) | 3,6 ms | 5,0 ms | 5,7 ms | 1,5 ms |
| 80 px | 8 (radial 8) | 6,9 ms | 8,5 ms | 9,2 ms | 2,5 ms |
| 80 px | 16 (miroir X × radial 8) | 7,9 ms | 9,7 ms | 10,3 ms | 3,4 ms |
| 80 px, Craie | 1 | 2,4 ms | 4,3 ms | 4,7 ms | 1,1 ms |
| 80 px, Éclaboussures | 1 | 2,0 ms | 3,2 ms | 4,9 ms | 1,0 ms |
| 80 px, Éclaboussures | 16 | 6,5 ms | 10,4 ms | 12,8 ms | 2,7 ms |
| 200 px | 8 | 11,8 ms | 13,5 ms | 15,1 ms | 4,8 ms |

**Budget de 8 ms par dab tenu en moyenne jusqu'à 16 copies** avec une brosse de 80 px. Le 95e centile le dépasse à partir de 8 copies, et une brosse de 200 px avec 8 copies le dépasse aussi. Le coût suit le nombre de tuiles touchées, multiplié par le nombre de copies : chaque copie peint sa propre région.

**Optimisation mesurée avant et après** (commit `perf(paint)`) :

| Cas (80 px) | Avant | Après |
| --- | --- | --- |
| 1 copie | 3,7 ms | 2,4 ms |
| 8 copies | 9,8 ms | 6,9 ms |
| 16 copies | 11,0 ms | 7,9 ms |

- **Première hypothèse, fausse** : le shader des dabs serait coûteux, puisque chaque texel projette les 16 copies. L'éviction des copies hors du cadre des dabs n'a fait gagner que 2 %. Elle est gardée, car elle est exacte.
- **Mesure** : le temps suivait le nombre de tuiles touchées. Un dab de 80 px sans symétrie en touchait 105, parce que les faces vues de dos, qui se projettent au même endroit, étaient comptées. Les exclure de façon exacte a ramené le dab à 2,4 ms.

**Autres temps** :
- début du trait : 2 ms sans symétrie, 9 ms avec 16 copies (une image de profondeur par copie) ;
- rejeu complet du trait dans un nouveau document : 90 ms sans symétrie, 370 ms avec 16 copies.

## Écarts constatés pendant l'implémentation

- **Tuiles touchées** : les triangles dont aucun texel ne fait face à la caméra sont exclus.
  - Le test est exact : la face d'un texel est une combinaison positive des termes nᵢ·t(pⱼ), entre les normales des sommets et les directions vers la caméra aux coins.
  - Les deux tests de tuiles comptaient comme peint un texel tourné vers l'arrière, que le shader ne peint pas (angle 0, comme `reference::texel_view`). Ils comptent désormais seulement les texels tournés vers la caméra, comme le shader.
  - La garantie ne change pas : tout texel qui reçoit de la peinture est dans une tuile touchée.
  - Un proptest et une vérification sur une grille de points du cube couvrent l'exclusion.
- **Shader des dabs** : une copie dont le pixel tombe hors du cadre des dabs du lot est ignorée avant son test de profondeur. Aucun dab n'y couvre rien (proptest), donc le résultat ne change pas.
- **Presets livrés** :
  - ils sont dans `patine_paint::presets`, pour que le bench puisse les utiliser ;
  - l'application garde les presets de l'utilisateur, en RON dans `%APPDATA%\Patine\presets` (ou `PATINE_PRESETS_DIR`) ;
  - un preset livré garde le matériau en cours, un preset de l'utilisateur l'enregistre.
- **Tablette** :
  - `emulate_tool_from_mouse(false)` : la souris passe par egui, pas par un outil émulé ;
  - les événements du stylet vont aux deux vues, et chacune n'accepte un appui que dans son cadre ;
  - un trait se termine sur l'appareil qui l'a commencé ;
  - l'inclinaison est interpolée le long du trait, comme la pression ;
  - backend obtenu au lancement : `WindowsInkRealTimeStylus`.
- **Graine des traits** : SplitMix64 du nombre de traits commencés. Elle change à chaque trait, mais elle suit la même suite à chaque session, donc les captures sont reproductibles.
- **Pipette** :
  - elle lit la valeur finale (composée) des canaux du texture set du triangle visé, un seul texel par canal (`readback::read_texel`) ;
  - pour un effet de peinture, elle lit le gris du masque du calque (`GpuDocument::mask_texture`) ;
  - dans la vue 2D, elle lit sous l'UV pointé du texture set courant.
- **Ligne droite** : elle part du dernier dab du trait précédent, seulement s'il a été peint avec la même caméra, ou le même cadrage de la vue 2D. Sinon, le clic commence un trait ordinaire.
- **Ligne de commande** :
  - `--stroke` prend une pression par point (`X,Y,P`) et les préfixes combinables `eraser:`, `uv:` (vue 2D) et `line:` (Maj + clic), au lieu de l'option `--stroke-uv` prévue ;
  - le bench prend `--symmetry N` (copies radiales) et `--mirror` (miroir X).
- **Éditeur de courbes** : double-clic pour ajouter un point, clic droit pour le retirer. Les extrémités restent en 0 et en 1. Les boutons « Linéaire » et « Désactivée » réinitialisent la courbe.
- **Hors du M9**, comme prévu :
  - l'extrémité gomme du stylet ne passe pas automatiquement à la gomme ;
  - pas d'isolation des dossiers ;
  - pas de déchargement CPU (M10).

## État à la clôture (14/10/2026)

- **Critère 1 (Wacom sous Windows)** : à vérifier à la main.
  - octotablet compile avec winit 0.30, et le service se connecte au lancement (`WindowsInkRealTimeStylus`).
  - La chaîne stylet → `PaintEvent` → dabs est testée par ses fonctions pures : suivi des trames, pression, inclinaison, coordonnées, choix entre stylet et souris, ligne droite.
  - Aucun stylet n'a été utilisé pendant le développement : la checklist (points 1 et 12) reste à faire.
- **Critère 2 (symétrie exacte sur la sphère)** : vérifié texel par texel.
  - Radial 3 sur la sphère : exact.
  - Miroir X sur le cylindre : exact (médiane < 1e-5, 99,5e centile < 1e-3).
  - Miroir X sur la sphère : médiane < 0,01 et 99e centile < 0,05. L'écart vient des diagonales des quads de la sphère UV, qui ne sont pas symétriques en X.
  - Un témoin sans symétrie échoue.
  - Le test passe sur WARP et sur la RTX 3080 Ti.
- **Critère 3 (rejeu déterministe avec jitter)** : vérifié à l'octet près.
  - Un trait peint en direct par lots aléatoires donne le contenu de son rejeu, avec jitters, scatter, suivi de trajectoire, pression et symétrie.
  - Deux rejeux sont identiques.
- **Format v2** : migration des manifestes et des strokes v1. Les fixtures v1 et v2 sont relues.
- **Références GPU = CPU** :
  - copies de symétrie, mode UV, plafond d'opacité et valeurs texturées ;
  - matériau procédural peint ;
  - normale d'ombrage, y compris sa mise à jour régionale.
- **Tests** : 326 passent sur WARP. Ceux de `patine_paint` et de `patine_compositor` passent aussi sur la RTX 3080 Ti.
- **À vérifier à la main** : la checklist ci-dessus, la tablette en premier, et le bench sur la RTX A2000.
