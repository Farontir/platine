# M8 — Peinture v1

Note de conception du milestone M8. Objectif : peindre un matériau directement sur le mesh.

Critères d'acceptation :
1. un trait qui traverse une couture UV ne laisse aucun trou sur le cube de test ;
2. rejouer le `StrokeLog` reproduit l'état peint en direct, à la tolérance près ;
3. le budget de latence est tenu : un dab en 4K avec 20 calques en moins de 8 ms sur la RTX 3080 Ti, et en moins de 16 ms sur la RTX A2000.

Le modèle de document a déjà ce qu'il faut depuis le M3 :
- `StrokeLog`, et `Stroke { brush: BrushSnapshot, tool, camera: StrokeCamera, symmetry, seed, dabs }` ;
- `LayerKind::Paint` et `MaskEffect::Paint` ;
- les strokes stockés à part dans le fichier (`strokes/<id>.bin`).

**Le format ne change pas** : `FORMAT_VERSION` reste 1 (voir la décision 9). Le moteur va dans le crate `patine_paint`.

## Décisions

1. **Contenu peint prémultiplié, avec couverture.**
   - **Une texture par (calque, canal) peint** :
     - `Rgba16Float` pour la couleur, l'émissif et la normale : rgb (ou xyz) prémultipliés, couverture dans `a` ;
     - `Rg16Float` pour les scalaires : valeur prémultipliée, couverture.
   - **Opérations linéaires** :
     - dépôt : `P' = P(1−a) + v·a` et `c' = c(1−a) + a` ;
     - gomme : `P' = P(1−a)` et `c' = c(1−a)`.
   - **Au mélange** : la source est dé-prémultipliée (`v = P / max(c, ε)`) et le poids devient `masque × opacité × c`.
     - `BlendStep` et `blend.wgsl` gagnent une option « source avec couverture », et la référence CPU fait le même calcul.
     - La couleur peinte a un alpha de 1 : l'opacité est un canal à part.
   - **Allocation et résidence** :
     - chaque texture est allouée au premier trait qui touche son canal ;
     - elle est **épinglée**, car seul un rejeu complet peut la recalculer ;
     - leur déchargement en mémoire CPU compressée passe au M10, avec les caches raster.
2. **Rastérisation par la carte de surface du M7**, au lieu de rendre les triangles en espace UV.
   - **Pour chaque texel des tuiles touchées**, le shader :
     - reconstruit le point de surface (position, normale) depuis la carte de surface ;
     - le projette avec la caméra du stroke ;
     - évalue chaque dab : disque avec dureté, rotation, alpha ;
     - teste l'occlusion contre la profondeur rendue depuis la caméra du stroke ;
     - atténue selon l'angle (`smoothstep` sur N·V).
   - **Pas de trou aux arêtes** : chaque centre de texel appartient à un seul triangle, sans rastérisation conservative.
   - **Dilatation gratuite** : les texels hors îlots prennent le point de surface le plus proche. C'est la dilatation à la composition demandée par le plan, et elle n'ajoute aucune donnée au document.
   - **Tuiles touchées** : le BVH, sur le CPU, donne les triangles dont la boîte écran touche un dab ; leurs boîtes UV donnent les tuiles.
3. **Un seul `rasterize_dabs`** (`patine_paint`), pour la peinture en direct comme pour le rejeu.
   - **Buffer de stroke `R32Float`** :
     - il accumule `s' = s + (1−s)·flow·alpha_dab` ;
     - cette accumulation est commutative, et le shader parcourt les dabs dans l'ordre ;
     - en f32, un trait découpé en lots donne donc exactement le même buffer que son rejeu d'un bloc.
   - **Contenu du calque** :
     - sur les tuiles touchées, `contenu = avant ⊕ (matériau, s × opacité)`, recalculé depuis l'instantané « avant » ;
     - le résultat ne dépend donc pas du découpage en lots, et l'opacité ne s'accumule pas à l'intérieur d'un trait.
   - **Profondeur d'occlusion** : une passe `shaders/paint/depth.wgsl`, depuis `StrokeCamera`, à la taille du viewport. Elle est rendue au début du trait, puis à nouveau au rejeu.
4. **Échantillonneur** (`patine_paint::sampler`, CPU pur).
   - Il reçoit les positions du pointeur, y compris les positions intermédiaires d'une image ; la pression vaut 1 pour la souris.
   - Il les lisse par lazy mouse (rayon réglable), puis place les dabs tous les `espacement × taille` pixels.
   - Les dabs enregistrés sont ceux d'après le lissage, donc le rejeu n'en dépend pas.
5. **Brosse et gomme, multi-canaux, alphas.**
   - **Matériau peint** : `BrushSnapshot.material = FillSource::Uniform`, une valeur par canal coché. Seuls ces canaux sont peints, ou gommés pour la gomme.
   - **Réglages** : taille en pixels écran, flow, opacité, dureté, espacement, angle, alpha.
   - **Reporté au M9** : taille en espace objet, jitters, scatter, pression.
   - **Alphas générés par code** (`patine_paint::alphas`) : rond, carré, éclaboussure, grain, trait, chacun avec sa licence (généré, CC0).
     - Un alpha utilisé est ajouté aux `resources` du projet, en PNG gris, par la commande du trait.
     - Le projet reste ainsi autonome.
6. **Peinture dans les masques**, annoncée par la note du M6.
   - Un effet `MaskEffect::Paint` a un contenu `Rg16Float` (valeur, couverture).
   - L'opération « texture » de `mask_effect.wgsl` tient compte de la couverture.
   - La brosse y peint une valeur de gris (blanc par défaut), et la gomme réduit la couverture.
7. **Undo par tuiles, côté GPU.**
   - **Commande** : `PaintStroke` ajoute ou retire le `Arc<Stroke>` du log, et le cœur reste sans pixels.
   - **Instantanés** :
     - pour chaque trait rastérisé, `GpuDocument` garde les tuiles « avant » qu'il a touchées ;
     - elles sont dans un atlas de tuiles, jetable dans le pool ;
     - l'atlas est indexé par l'identité du `Arc<Stroke>`.
   - **À la synchronisation**, on compare le log du document au log rastérisé (préfixe commun) :
     - **undo** : les tuiles « avant » sont restaurées, en ordre inverse ;
     - **redo** : seul ce trait est rastérisé à nouveau ; il repart du même état, donc le résultat est exact ;
     - **instantané évincé** : le calque est rejoué entièrement, plus lentement mais correctement.
   - **Écart** : PLAN.md met les instantanés dans la commande. Ici, `patine_core` reste sans pixels, et la mémoire des instantanés passe par le budget du pool.
8. **Composition par tuiles sales.**
   - **Pendant un trait** : `GpuDocument::paint` rastérise les nouveaux dabs, puis recompose les canaux touchés à partir du calque peint, sur les seules tuiles touchées. Le cache inférieur sert, et les mips ne sont refaits que sur la région.
   - **En fin de trait** : la commande ajoute au log le `Arc<Stroke>` peint en direct. `sync` le reconnaît, et n'a rien à rastériser.
   - **Undo et redo** : seules les tuiles restaurées sont recomposées.
   - **Autres changements** : évaluation complète, comme avant.
   - **Masques** : leur évaluation devient régionale, elle aussi.
9. **`StrokeLog.strokes: Vec<Arc<Stroke>>`**, avec la feature `rc` de serde.
   - **Format inchangé** : un `Arc<T>` est sérialisé exactement comme `T`. La fixture v1 est relue en test.
   - **Coût d'un trait ajouté** : sans `Arc`, `Arc::make_mut` recopierait tous les dabs du log.
   - **Comparaison** : `PartialEq` court-circuite par `Arc::ptr_eq`, pour ne pas comparer des milliers de dabs à chaque image.
   - **Identité** : l'`Arc` sert aussi d'identité aux instantanés.
10. **Curseur de brosse.**
    - Le picking du survol, déjà asynchrone, donne la position et le triangle ; on en déduit la normale.
    - L'app trace avec le painter d'egui un cercle 3D orienté par la normale, projeté à la taille réelle de la brosse.
    - Le cercle est gris quand la sélection ne se peint pas.
11. **Cible de peinture** : le calque paint sélectionné, ou l'effet de masque paint sélectionné.
    - On les crée par « Nouveau calque de peinture » (panneau Calques) et par « Ajouter de la peinture » (menu du masque).
    - Avec une autre sélection, le trait est refusé, avec un message.
12. **Hors du M8**, écarts à noter :
    - peindre un matériau procédural : M9, avec les presets ;
    - isoler les dossiers, ce que la couverture permettrait maintenant ;
    - projection, clone et smudge : outils ultérieurs ;
    - déchargement CPU : M10.

## Structures

- **`patine_core`** :
  - `StrokeLog { strokes: Vec<Arc<Stroke>> }` ;
  - `PaintTarget { Layer(LayerId), Mask(LayerId, EffectId) }` ;
  - la commande `PaintStroke { target, stroke, alpha: Option<(ResourceHash, Resource)> }` ;
  - les constructeurs `Layer::paint` et `MaskEffect::paint`.
- **`patine_paint`** :
  - `StrokeSampler` (`sampler.rs`) ;
  - `Alpha` et `ALPHAS` (`alphas.rs`) ;
  - `touched_tiles` (`tiles.rs`) ;
  - `Painter`, `rasterize_dabs`, `StrokeTarget` (`raster.rs`) ;
  - le miroir CPU (`reference.rs`).
- **Shaders** (`shaders/paint/`) :
  - `depth.wgsl` : profondeur depuis la caméra du stroke ;
  - `dabs.wgsl` : accumulation dans le buffer de stroke ;
  - `stroke_composite.wgsl` : avant ⊕ stroke, et la gomme.
- **`patine_compositor`** :
  - `Source` avec couverture ;
  - `PaintRasters` (textures par canal, log rastérisé, atlas « avant ») ;
  - `GpuDocument::{begin_stroke, paint, end_stroke}` ;
  - `Evaluation.region` et `evaluate_mask` par région.
- **Application** :
  - `ToolState` : outil actif et réglages de brosse, un état d'interface ;
  - actions `ToolBrush` (B), `ToolEraser` (E), `BrushSmaller` et `BrushLarger` ([ et ]), `SofterBrush` et `HarderBrush` (Maj + [ et ]), `NewPaintLayer` ;
  - section « Brosse » des Propriétés ;
  - curseur de brosse.

## Tests

- **Cœur** :
  - apply et revert de `PaintStroke` ;
  - proptests d'undo avec des traits aléatoires ;
  - aller-retour de fichier ;
  - fixture v1 relue.
- **Échantillonneur** : écart entre dabs égal à `espacement × taille`, lissage borné, aucun dab perdu entre deux images (proptest).
- **Rastérisation** :
  - GPU = CPU par texel, à 1/512 ;
  - profondeur comparée au BVH.
- **Critère 1** :
  - un trait horizontal traverse les coutures du cube (`primitives::cube_cross`) ;
  - pour chaque pixel écran du cœur du trait qui touche le mesh, on lit le contenu peint à son UV, avec un filtrage bilinéaire comme le viewport : la couverture doit valoir au moins 0,99 ;
  - un témoin sans dilatation doit, lui, montrer des trous, ce qui prouve que le test les détecte.
- **Critère 2** :
  - un trait découpé en lots aléatoires donne un buffer et un contenu identiques à son rejeu d'un bloc ;
  - dans `GpuDocument`, l'état peint en direct égale le rejeu après rechargement du projet.
- **Compositeur** :
  - pile avec calques peints, masque peint et dossier : GPU = référence par texel ;
  - undo et redo : restauration sans rejeu (compteurs), puis état identique ;
  - instantané évincé : même résultat ;
  - tuiles hors région intactes.
- **Critère 3** : `patine_cli bench paint`.

## Risques

- **Coût par dab** : il additionne les tuiles touchées (16 384 texels chacune) pour chaque dab du lot, les mélanges de 20 calques × 5 canaux sur ces tuiles, puis les mips régionaux. Si la mesure dépasse 8 ms :
  - limiter le lot par image ;
  - grouper les tuiles dans un seul dispatch par canal ;
  - repousser les mips en fin de trait.
- **Occlusion aux silhouettes** : un biais relatif à la distance, réglé par test sur la sphère et le cube.
- **Mémoire** :
  - un calque paint 4K sur 5 canaux pèse 448 Mo, plus 64 Mo de carte de surface et les atlas « avant » ;
  - l'allocation se fait par canal touché, et le bench mesure la mémoire ;
  - le déchargement CPU attend le M10.
- **Seuil de N·V** : il évite les étirements rasants, mais ne doit pas créer de trou au critère 1. Le test utilise une vue où les faces traversées gardent un angle raisonnable, et le seuil est documenté.

## Checklist de tests manuels

1. Casque : Nouveau calque de peinture, puis peindre à la souris. Le trait suit le curseur, sans trou aux coutures, et l'arrière du mesh n'est pas peint.
2. Valise : un trait qui traverse plusieurs îlots UV reste continu.
3. Couleur et roughness cochées, puis peindre : les deux canaux changent (touche C pour les voir).
4. Gomme (E) sur une partie du trait : la peinture disparaît, et le calque du dessous réapparaît.
5. Changer d'alpha et de dureté ; [ et ] changent la taille, Maj + [ et ] la dureté.
6. Le curseur suit la surface, orienté selon la normale, à la taille du trait.
7. Ctrl+Z et Ctrl+Y sur plusieurs traits : instantanés et exacts.
8. Enregistrer puis rouvrir : identique.
9. Masque noir sur un fill, Ajouter de la peinture, puis peindre en blanc : le fill apparaît sous le trait (touche M pour voir le masque).
10. `patine_cli bench paint` sur la RTX A2000 : moins de 16 ms par dab.
