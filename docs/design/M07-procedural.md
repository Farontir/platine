# M7 — Bibliothèque procédurale

Note de conception du milestone M7. Objectif : des fills procéduraux paramétrables, avec une interface générée.

Critères d'acceptation :
1. chaque bruit est tileable (bords identiques à 1e-3 près) ;
2. la projection triplanaire ne montre pas de couture sur la sphère de test ;
3. un paramètre modifié se voit en moins de 100 ms.

Le modèle de document a déjà ce qu'il faut depuis le M3 :
- `FillSource::Procedural(ProceduralRef { id, version, params })` ;
- `Projection { kind, scale, rotation, offset }`, où `kind` vaut UV, triplanaire (avec fondu), planaire, sphérique ou cylindrique ;
- `ParamValue`.

**Le format ne change pas** : `FORMAT_VERSION` reste 1. Le code va dans le crate `patine_procedural`.

## Décisions

1. **Définitions en données Rust.**
   - Ce sont `ProceduralDef { id, version, name_key, category, params, shader, outputs }` et `ParamDef { name, label_key, kind }`, dans `patine_procedural`. Elles sont sérialisables pour être chargées de fichiers à partir du M16 ; `patine_core` ne change pas.
   - `ParamKind` vaut `Float { min, max, default }`, `Int { min, max, default }`, `Seed`, `Bool`, `Color`, `Enum { options }`, `Curve`, `Gradient` ou `Texture`.
   - Au M7, les shaders utilisent float, int, graine, booléen, couleur et enum. Courbe, dégradé et texture sont décrits, mais le calcul du layout les refuse jusqu'à leur milestone (filtres au M15, bibliothèque au M16).
2. **Layout du uniform buffer.**
   - `layout(def)` calcule les offsets selon les règles d'alignement WGSL des uniforms, et `pack(def, params)` écrit les octets.
   - Chaque shader déclare à la main sa structure `Params` (aucun shader inline). Un test lit chaque shader avec naga et vérifie que les noms, les types et les offsets sont ceux du layout calculé.
3. **Paramètres et versions.**
   - `resolve(def, &ProceduralRef)` donne les valeurs effectives : défaut pour un paramètre absent, paramètre inconnu ignoré, valeur bornée à son domaine, migration selon la version (au M7, la version est simplement mise à jour).
   - Un id inconnu de la bibliothèque ne contribue pas, et un avertissement s'affiche dans la console.
4. **Graine et tileabilité.**
   - Tous les bruits partent d'un hash entier (PCG) identique en WGSL et en Rust ; la graine décale le hash.
   - Chaque bruit est périodique sur une grille entière : `noise(p + période) = noise(p)`. Les octaves de fBm doublent la période et restent donc périodiques.
   - En projection UV, une échelle entière rend le fill tileable.
   - Le simplex périodique utilise un réseau incliné à périodes entières, d'après Gustavson et McEwan (JCGT 2022, « Tiling simplex noise »). L'implémentation est écrite à partir de l'article.
5. **Carte de surface par texture set** (`SurfaceMap`, partagée par tous les procéduraux du set).
   - Une passe rastérise les triangles du set dans l'espace UV et écrit l'indice de chaque triangle dans une texture `R32Uint`.
   - Un *jump flood* donne ensuite à chaque texel non couvert le triangle couvert le plus proche.
   - Le shader procédural reconstruit, pour son texel, le point le plus proche sur ce triangle (barycentriques bornées), et interpole position, normale et tangente depuis les buffers du mesh.
   - Les bords d'îlots sont ainsi dilatés vers le point de surface le plus proche, sans couture au filtrage ni aux mips.
   - La carte n'est refaite qu'au changement de résolution. Elle est jetable et servira aussi au baker (M11).
6. **Projections**, chacune avec sa référence CPU, l'échelle, la rotation et le décalage du `Projection` :
   - UV ;
   - triplanaire : trois plans fondus selon la normale, `blend` règle la netteté du fondu ;
   - planaire : le plan face à +Z du monde ;
   - sphérique et cylindrique (axe Y) autour du centre de la boîte englobante du mesh.

   Le format ne porte pas d'axe pour la projection planaire : en ajouter un serait un changement de format.
7. **Matériaux.**
   - Un matériau produit les canaux standards qu'il déclare dans `outputs`. Un calque n'affecte que les canaux communs au matériau et au texture set.
   - La normale vient du gradient de la hauteur procédurale (différences finies), ramené en espace tangent avec le repère interpolé (T, B = signe × N × T, N). En projection triplanaire, le gradient est pris en espace monde.
   - Une passe par canal pour commencer : le shader calcule le matériau et rend le canal demandé. On regroupera en MRT seulement si la mesure dépasse le budget.
8. **Les 5 premiers matériaux** :
   - acier brossé : rayures étirées, anisotropie simulée par la roughness ;
   - cuivre : métal, variations de teinte et de roughness ;
   - plastique grainé : grain Worley en relief ;
   - bois : cernes déformés par du bruit, fibres ;
   - béton : fBm, pores Worley, taches.

   Chacun a sa référence CPU.
9. **Bruits et motifs**, comme procéduraux gris utilisables en fill de masque (`MaskEffect::Fill { source: Procedural }`) :
   - bruits : value, Perlin, simplex, Worley (F1, F2, F2 − F1), fBm, ridged, turbulence, domain warp ;
   - motifs : dégradé, damier, tuiles, briques, rayures, cercles, rayures aléatoires, fibres.

   Les sources image restent pour l'import d'images (M16) ; la note du M6 les annonçait au M7.
10. **Cache de contenu.**
    - Il y a une texture par couple (calque, canal) procédural et par effet de masque procédural, jetable.
    - Elle n'est recalculée que si la source, la projection ou la résolution change.
    - La passe de mélange la lit comme source (`Source::Texture`), et la passe d'effet de masque gagne une source texture.
11. **Bibliothèque.**
    - Le panneau Bibliothèque liste les matériaux, et les bruits et motifs dans la catégorie « Masques », avec une vignette. La vignette est rendue une fois par le même shader sur une surface plane et éclairée simplement par sa normale ; les vignettes sur sphère viendront au M16.
    - Un double-clic sur un matériau l'ajoute en calque au-dessus de la sélection. Les bruits et motifs s'ajoutent à un masque par son menu contextuel.
12. **Interface générée.**
    - Le panneau Propriétés construit un éditeur par `ParamDef` : curseur pour float et int, case pour bool, couleur, liste pour enum, champ de graine avec un bouton dé.
    - Une section Projection propose le type, l'échelle, la rotation, le décalage et le fondu triplanaire.
    - Les gestes fusionnent en une entrée d'historique.

## Structures

- `patine_core` :
  - commandes `SetFillSource { layer, source }` et `SetProjection { layer, projection }`, sur le motif *swap* et fusionnables par calque ;
  - constructeur `Layer::fill_procedural`.
- `patine_procedural` :
  - `def` (définitions, layout, pack, resolve) et `library` (les 21 définitions) ;
  - `reference` : miroir CPU du hash, des bruits, des motifs, des projections, de la surface et des matériaux ;
  - `surface` : buffers du mesh par set, carte de surface ;
  - `evaluate` : pipelines et passes d'évaluation.
- `shaders/procedural/` :
  - bibliothèque : `hash`, `noise`, `pattern`, `projection`, `surface` ;
  - passes : `surface_raster`, `surface_jfa` ;
  - un fichier d'entrée par procédural (`materials/`, `noises/`, `patterns/`).
- `patine_compositor` : carte de surface et cache de contenu dans `GpuDocument`, opération « texture » de la passe d'effet de masque, référence évaluable par texel.

## Tests

- Layout naga de la structure `Params` de chaque shader.
- **Critère 1** : pour chaque bruit, à plusieurs échelles entières et graines, `f(0, v) = f(1, v)` et `f(u, 0) = f(u, 1)` à 1e-3 près, sur le GPU et sur le CPU.
- GPU = CPU sur des points aléatoires (1e-4) pour les bruits, motifs, projections et matériaux. Les graines changent le résultat.
- Carte de surface : chaque texel couvert est dans son triangle, et un texel non couvert pointe vers un triangle à au plus un texel de la distance minimale.
- **Critère 2** : un bruit en triplanaire sur la sphère de test, échantillonné des deux côtés de chaque arête de couture UV, a un écart inférieur à un seuil tiré de son gradient. Contrôle : en UV, une échelle non entière fait échouer le même test, ce qui prouve qu'il détecte bien une couture.
- Piles avec fills et masques procéduraux : GPU = référence par texel. Changer un paramètre ne recalcule que ce contenu et les mélanges à partir du calque ; l'undo redonne l'état antérieur.
- **Critère 3** : `patine_cli bench procedural`.

## Risques

- **Simplex périodique** : l'algorithme est délicat. Après trois essais infructueux, rapport et repli : Perlin sur réseau triangulaire, ou report après le MVP.
- **Écarts CPU/GPU** : pas de fonction transcendante dans les hash ; tolérance de 1e-4 pour les primitives et de 1/512 pour les compositions stockées en f16.
- **Coût du recalcul** : 5 passes par fill procédural en 4K, avec du fBm (estimation : 10 à 20 ms sur la RTX 3080 Ti). Repli : MRT, ou moins d'octaves en aperçu.
- **Mémoire** : un contenu par canal procédural, plus la carte de surface (64 Mo par set en 4K), tous jetables.

## Checklist de tests manuels

- [ ] Bibliothèque : chaque matériau a une vignette ; un double-clic l'ajoute en calque sur le casque et sur la sphère.
- [ ] Régler les paramètres d'un matériau : le rendu suit en direct, une entrée d'historique par geste.
- [ ] Changer la graine (bouton dé) : le motif change, la couleur moyenne reste.
- [ ] Projection triplanaire contre UV sur la valise : pas de couture en triplanaire, l'étirement des UV disparaît.
- [ ] Clic droit sur un masque › Ajouter un fill procédural › un bruit : le calque se découvre par taches ; touche M pour voir le masque.
- [ ] Ctrl+Z et Ctrl+Y sur chaque opération ; enregistrer, rouvrir : identique.
- [ ] `cargo run --release -p patine_cli -- bench procedural` sur la RTX A2000.
