# M5 — Compositeur v1

Note de conception du milestone M5. Objectif : une pile de calques fill complète et rapide.

Critères d'acceptation :
1. chaque blend mode correspond à la référence CPU à 1/512 près ;
2. le budget « paramètre d'un fill » est tenu : changer un paramètre d'un fill en bas d'une pile de 30 calques 4K prend moins de 100 ms sur la RTX 3080 Ti, moins de 250 ms sur la RTX A2000.

## Décisions

1. **Passes de rendu plutôt que compute.** Chaque mélange est une passe plein écran qui lit la composition courante (texture A) et écrit la suivante (texture B), en alternance (*ping-pong*). Cela marche pour `Rgba16Float` et `R16Float` sans la feature de stockage `R16Float`, et les tuiles sales deviennent des rectangles de *scissor*. Un fill uniforme est une constante passée en uniform : aucune texture de contenu au M5 (le cache de contenu viendra avec les procéduraux et la peinture).
2. **Formule.** Pour chaque canal, en partant de la valeur par défaut, chaque calque visible applique `dst = mix(dst, blend(dst, C), M × α)`, avec C le contenu, M le masque et α l'opacité du canal. Le masque vaut 1 jusqu'au M6 ; la passe lit déjà une texture de masque (1×1 blanche).
3. **Dossiers.** Un dossier compose ses enfants sur la composition du dessous (dans une texture temporaire qui en part), puis mêle le résultat à la composition avec son opacité par canal : à 50 %, tout le groupe s'atténue ; à 100 %, il équivaut à ses enfants posés directement. Sans couverture alpha par pixel (elle arrive avec la peinture au M8), un autre mode appliquerait deux fois la composition du dessous (Multiply donnerait `dst × (dst + enfants)`) : pour un dossier, Normal et Passthrough sont donc équivalents, Disable ignore le dossier, et les autres modes se comportent comme Normal. Le M8 pourra introduire l'isolation sans changer le format. *(Précision apportée en écrivant la référence CPU : le plan prévoyait que le dossier se mélange avec son propre mode.)*
4. **Canal normal.** Normal Map Combine (*Reoriented Normal Mapping*, mode par défaut) et Normal Map Detail (UDN). Normal et Replace interpolent puis renormalisent, Disable n'applique rien ; les autres modes n'ont pas de sens sur des vecteurs et se comportent comme Normal.
5. **Modes non séparables** (Hue, Saturation, Color, Luminosity) : formules de la spécification W3C *Compositing and Blending* (Lum, Sat, ClipColor). Sur un canal scalaire, ils opèrent sur le gris (v, v, v).
6. **Invalidation par différence.** Comme au M4, `GpuDocument` compare le document à ce qu'il a évalué (une copie de la pile et des canaux de chaque set). Le premier calque de premier niveau qui diffère donne le point de départ de l'évaluation.
7. **Cache inférieur.** Pour chaque canal du set en cours d'édition, la composition sous le calque de départ est gardée. Tant que les modifications portent sur ce calque ou au-dessus, seuls ce calque et ceux du dessus sont remélangés ; le cache est refait quand le point de départ descend. Les caches et les textures de *ping-pong* sont jetables dans le pool.
8. **Tuiles sales.** Une `Region` (ensemble de tuiles de 128 × 128) accompagne chaque évaluation. Les fills salissent toute la texture, mais le chemin par tuiles est implémenté et testé pour la peinture du M8.
9. **Mips.** Après chaque évaluation, une passe de réduction 2 × 2 (moyenne, normales renormalisées) remplit les niveaux, avec sa référence CPU.
10. **Nouveau calque fill.** Il affecte tous les canaux actifs du texture set, avec leurs valeurs par défaut et le mélange standard : le rendu ne change pas tant qu'on ne règle rien.

## Structures

- `patine_core` : commande `SetFillValue { layer, channel, value: Option<[f32; 4]> }` (fusionnable par calque et canal ; `None` retire le canal du contenu) ; constructeurs `Layer::fill_for` et `Layer::folder`.
- `patine_compositor::reference` : `blend(mode, kind, dst, src)`, `composite(stack, channel, default)`, `downsample`.
- `shaders/compositor/` : `blend_ops.wgsl` (tous les modes), `blend.wgsl` (passe de mélange), `copy.wgsl`, `downsample.wgsl`.
- `patine_compositor::Compositor` : pipelines, *ping-pong*, évaluation d'un canal d'un set sur une `Region`.
- `GpuDocument::sync` : allocation (M4), diff des piles, évaluation à partir du calque de départ, mips ; renvoie les sets dont les bind groups changent.

## Application

- Panneau Calques : pile du set courant du haut vers le bas, dossiers repliables, œil de visibilité, sélection, double-clic pour renommer, glisser-déposer (dans et hors des dossiers), boutons Nouveau fill, Nouveau dossier, Supprimer. En tête, le canal courant avec le mode et l'opacité de ce canal pour le calque sélectionné.
- Panneau Propriétés : pour le fill sélectionné, chaque canal avec sa valeur (couleur, curseur, ou x, y, z pour la normale), son activation, son mode et son opacité.
- Raccourcis : Suppr (supprimer le calque), Ctrl+G (grouper dans un dossier).

## Tests

- Référence CPU : valeurs connues de chaque mode ; propriétés (Normal à opacité 1 donne le fill, opacité 0 donne la destination, normales unitaires).
- **Critère 1** : pour chaque mode et chaque type de canal, destination aléatoire, plusieurs fills et opacités : GPU = référence CPU à 1/512 près.
- Tuiles : seules les tuiles demandées changent.
- Piles aléatoires (proptest, petites résolutions) : GPU = `reference::composite`, dossiers et passthrough compris.
- Cache inférieur : une modification ne remélange que depuis le calque modifié (nombre de passes) ; undo redonne l'état précédent.
- Mips : GPU = réduction CPU.
- **Critère 2** : `patine_cli bench compositor`.

## Risques

- Budget : 30 calques × 5 canaux ≈ 150 passes 4K. Estimation 40 à 60 ms sur la RTX 3080 Ti, plus serrée sur la RTX A2000. Repli si la mesure dépasse : regrouper les fills sans masque dans une passe.
- Précision f16 près de 0 pour Divide et Color Dodge : la référence arrondit ses entrées en f16 comme le GPU.
- Mémoire des caches du set édité (≈ 7 × 3 textures 4K pour le casque), jetables.

## Checklist de tests manuels

- [ ] Créer des fills et des dossiers ; glisser-déposer dans et hors des dossiers ; renommer ; supprimer (bouton et Suppr) ; Ctrl+G.
- [ ] Masquer un calque (œil) : le rendu et la vue 2D suivent.
- [ ] Régler les valeurs d'un fill, ses modes et opacités par canal : le rendu suit en direct, une entrée d'historique par geste.
- [ ] Modes Multiply, Screen, Overlay sur la couleur ; Normal Map Combine sur la normale : rendu cohérent.
- [ ] Dossier à opacité 50 % : tout le groupe s'atténue ; Passthrough : les enfants agissent directement.
- [ ] Ctrl+Z et Ctrl+Y sur chaque opération.
- [ ] Enregistrer, rouvrir : pile identique.
- [ ] `cargo run --release -p patine_cli -- bench compositor` sur la RTX A2000 : moins de 250 ms.

## Mesures (11/10/2026, RTX 3080 Ti, build release)

`cargo run --release -p patine_cli -- bench compositor` : sphère, un texture set à 5 canaux, des fills aux modes variés (Normal, Multiply, Overlay, Screen, Soft light ; Normal Map Combine sur la normale), opacité 70 %. On modifie le fill du bas 20 fois, avec l'attente GPU et les mips comprises.

| Pile | Première évaluation | Valeur (couleur de base) | Visibilité (5 canaux) |
| --- | --- | --- | --- |
| 30 fills, 4096² | 55 ms | 7,9 ms (max 8,7) | 32,7 ms (max 34,0) |
| 30 fills, 2048² | 36 ms | 2,8 ms (max 3,4) | 12,5 ms (max 13,3) |
| 60 fills, 4096² | 93 ms | 14,4 ms (max 15,5) | 63,7 ms (max 65,1) |

Critère 2 tenu : 7,9 ms pour un budget de 100 ms. Le pire cas, la visibilité qui touche les 5 canaux, reste à 33 ms. Changer une valeur ne réévalue que le canal concerné. L'optimisation prévue en repli (regrouper les fills dans une passe) n'est donc pas nécessaire. Il reste à mesurer sur la RTX A2000 (budget 250 ms).

## Écarts constatés pendant l'implémentation

- **Dossiers** (décision 3) : ils composent leurs enfants sur la composition du dessous puis se mêlent en Normal avec leur opacité. Passthrough donne le même résultat ; Disable ignore le dossier.
- **`CommandGroup`** : une suite de commandes forme une seule entrée d'historique, et les commandes déjà appliquées sont annulées si l'une échoue. Elle sert à activer un canal (mélange et valeur) et à grouper dans un dossier (création puis déplacement).
- **`clip_color`** : sur un gris pur, la formule W3C divise par zéro (NaN sur le CPU comme sur le GPU). Une garde `> 1e-4` laisse la couleur inchangée dans ce cas.
- **Formule de `mix`** : la référence CPU calcule `a·(1−w) + b·w`, comme le `mix` de WGSL, et non `a + (b−a)·w`. Les deux écritures divergeaient d'un ulp f16 après une vingtaine de calques.
- **Piles aléatoires** : le test GPU = CPU sur des piles de 2 à 8 calques exclut deux familles de modes, dont chaque mode reste couvert seul par le test du critère 1.
  - Les modes qui amplifient l'écart d'arrondi f16 (Divide, Color dodge, Color burn : dérivée non bornée près de 0 ou de 1).
  - Les modes non séparables (Hue, Saturation, Color, Luminosity : teinte discontinue près des gris).
- **Disable sur un canal scalaire** : le résultat est `[r, 0, 0, 0]`, comme une texture `R16Float` relue.
- **`GpuDocument::sync`** : le cache inférieur est invalidé quand le point de départ est le premier calque. Une nouvelle valeur par défaut ne signale plus le set, car seules les textures remplacées changent les bind groups.
- **Options de capture** : `--edit fill=R,G,B,RUGOSITÉ` (nouveau fill sélectionné au-dessus de la sélection), `--edit mode=MODE,OPACITÉ` (mélange de la couleur de base du calque sélectionné) et `--edit group=`.
- **Bench** : `--edits N` remplace le `--channels` du plan, car le nombre de canaux est celui du texture set par défaut.

## État à la clôture (11/10/2026)

- Critère 1 : pour les 25 modes et les 3 types de canal, la passe GPU égale la référence CPU à 1/512 près, sur WARP et sur la RTX 3080 Ti. Les tuiles hors région restent intactes et les mips égalent la réduction CPU.
- Piles aléatoires (dossiers, passthrough, visibilité, Disable) : GPU = `reference::composite_stored`, avec 0 échec sur 40 exécutions de 24 piles sur chaque adaptateur.
- Cache inférieur : les nombres de passes attendus (10, 3, 3, 10, 8) sont exacts. L'undo redonne l'état précédent.
- Critère 2 : 7,9 ms sur la RTX 3080 Ti.
- 210 tests passent sur WARP.
- **À vérifier à la main** : la checklist ci-dessus, et le bench sur la RTX A2000.
