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
11. `patine_cli bench paint --symmetry 8` sur la RTX A2000 : moins de 16 ms par dab.
