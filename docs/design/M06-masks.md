# M6 — Masques

Note de conception du milestone M6. Objectif : masquer n'importe quel calque avec une pile d'effets.

Critères d'acceptation :
1. les masques composés correspondent à la référence CPU ;
2. chaque opération s'annule.

Le modèle de données existe depuis le M3 (`patine_core::mask`). Un `MaskStack` a une valeur initiale, `Black` ou `White`, et des effets du bas vers le haut. Chaque `MaskEffectEntry` porte un identifiant stable, une visibilité, un blend mode, une opacité et son opération. Le M6 évalue les opérations `Fill`, `Levels` et `Filter { kind: Invert }`. **Le format ne change pas** : `FORMAT_VERSION` reste 1, et la fixture v1 est toujours relue.

## Décisions

1. **Sémantique d'un effet.** Le masque part de 0 (noir) ou de 1 (blanc). Chaque effet visible applique, de bas en haut, `m = mix(m, blend(m, e), opacité)` avec les blend modes scalaires du M5. Pour un fill, e est sa valeur. Pour Levels et Invert, e est la transformation du masque du dessous : `levels(m)` ou `1 − m`. En Normal à 100 %, ces deux effets remplacent donc le masque par sa transformation ; à 50 %, ils en donnent la moitié. Le résultat est borné à [0, 1].
2. **Levels.** `x = clamp((m − in_low) / max(in_high − in_low, 1e-4), 0, 1)`, puis `x = x^(1/gamma)`, puis `out_low + x × (out_high − out_low)`. Les points de sortie peuvent être inversés (`out_low > out_high`), ce qui inverse le masque.
3. **Valeur d'un fill de masque.** Elle est stockée dans `FillSource::Uniform`, sous la clé `Channel::Opacity`, dans la première composante. C'est une convention documentée dans `mask.rs`, qui ne change pas le format. À la lecture, on prend la première entrée de la table : la fixture v1, qui utilise `User(0)`, reste valide. Les sources Image et Procedural arrivent au M7.
4. **Effets pas encore pris en charge** (Paint, Generator, les autres filtres, AnchorRef, PolygonFill) : ils laissent le masque inchangé, dans la référence comme sur le GPU. L'interface ne permet pas de les créer avant leur milestone.
5. **Texture de masque.** Chaque calque masqué a une texture `R16Float` à la résolution du set, avec ses mips pour l'affichage. Elle est jetable dans le pool, car le masque est recalculable, et n'est réévaluée que quand son `MaskStack` change : c'est le cache de masque de PLAN.md. Les masques restent uniformes jusqu'au M7 (procéduraux) et au M8 (peinture), mais le chemin est déjà par pixel.
6. **Composition.** La passe de mélange du M5 lit la texture de masque du calque à la place de la texture blanche : `dst = mix(dst, blend(dst, C), M × α)`. Un dossier masqué fait `mix(dessous, résultat du dossier, M × α)`, ce qui prolonge la décision 3 du M5. Ajouter, modifier ou retirer un masque rend le calque différent sur tous ses canaux : `first_difference` en fait le point de départ, et le cache inférieur reste valable en dessous.
7. **Passes.** Une passe initiale remplit le masque de 0 ou de 1. Les fills de masque réutilisent la passe de mélange du M5, en type scalaire. Une nouvelle passe, `shaders/compositor/mask_effect.wgsl`, applique Levels ou Invert au masque du dessous puis le mêle avec `blend_ops.wgsl`.
8. **Mode d'affichage « Masque ».** `DisplayMode::Mask` réutilise le chemin du canal isolé (texture inspectée, en niveaux de gris). L'application lie la texture de masque du calque sélectionné ; s'il n'a pas de masque, elle lie une texture blanche (tout visible). Les autres texture sets s'affichent en noir. La vue 2D suit le même mode. Le libellé du mode, en haut à gauche de la vue, donne le nom du calque ou indique « aucun masque ». L'action « Afficher le masque » (touche M) est dans le registre et dans Paramètres d'affichage.
9. **Sélection.** La sélection du panneau Calques devient `Selection { layer, part }`, où `part` vaut `Content`, `Mask` ou `MaskEffect(EffectId)`. Le panneau Propriétés montre la partie sélectionnée.

## Structures

- `patine_core::commands` :
  - `SetMask { layer, mask: Option<MaskStack> }` : ajouter un masque noir ou blanc, le retirer, changer sa valeur initiale ;
  - `InsertMaskEffect { layer, index, entry }` : consomme un `EffectId` et restaure `next_id` à l'annulation ;
  - `RemoveMaskEffect { layer, effect }`, `MoveMaskEffect { layer, effect, index }` ;
  - `SetMaskEffect { layer, effect, entry }` : visibilité, mode, opacité, valeur, levels. Elle est fusionnable par (calque, effet), si bien qu'un geste de curseur donne une seule entrée d'historique.
- `patine_core::mask` : `MaskStack::new(initial)`, `MaskEffect::uniform(v)`, `MaskEffect::fill_value()`, `MaskEffect::invert()`.
- `patine_compositor::reference` : `levels`, `mask_effect`, `mask`, `mask_stored`. `composite` et `composite_stored` tiennent compte du masque (`poids = M × α`).
- `patine_compositor::Compositor` : passe d'effet de masque et passe de valeur initiale, avec un compteur de passes.
- `GpuDocument` : les textures de masque de chaque set, indexées par calque et réévaluées quand le stack change. `mask(set, layer)` donne la vue du masque d'un calque. `sync` signale aussi les sets dont une texture de masque a été réallouée.
- `patine_render::DisplayMode::Mask`.

## Application

- **Panneau Calques.**
  - Sous un calque masqué, une ligne « Masque » (noir ou blanc), puis ses effets du haut vers le bas, chacun avec son œil.
  - Menu contextuel d'un calque : « Ajouter un masque noir », « Ajouter un masque blanc », « Supprimer le masque ».
  - Menu contextuel du masque ou d'un effet : « Ajouter un fill », « Ajouter un levels », « Ajouter une inversion », « Supprimer l'effet ».
  - Glisser-déposer pour réordonner les effets dans leur masque. Suppr supprime l'effet ou le masque sélectionné.
- **Panneau Propriétés.** Pour le masque : sa valeur initiale. Pour un effet : visibilité, mode, opacité, puis sa valeur (fill) ou les 5 curseurs de levels avec un aperçu de la courbe.
- **Affichage.** Mode « Masque » (touche M), dans Paramètres d'affichage et dans le menu Affichage.
- **Options de capture.**
  - `--edit mask=black|white` agit sur le calque sélectionné.
  - Les effets s'ajoutent avec `--edit mask-fill=V,MODE,OPACITÉ`, `--edit mask-levels=IN_LOW,IN_HIGH,GAMMA,OUT_LOW,OUT_HIGH` et `--edit mask-invert=`.
  - `--display mask` choisit le mode d'affichage.

## Tests

- Commandes : `apply` puis `revert` redonne le projet identique. Les proptests d'undo s'étendent aux opérations aléatoires sur les masques.
- Référence CPU : valeurs connues (levels, inversion, fills dans chaque mode), propriétés (opacité 0 sans effet, double inversion = identité, levels par défaut = identité, résultat dans [0, 1]).
- **Critère 1** :
  - chaque effet, sur une texture de masque aléatoire, dans chaque mode scalaire et à plusieurs opacités : GPU = référence à 1/512 près ;
  - piles aléatoires de calques masqués (dossiers compris) : GPU = `composite_stored`.
- Cache : modifier un masque ne réévalue que ce masque, puis les mélanges à partir de son calque (compteurs de passes). Undo redonne l'état précédent, et les textures de masque sont libérées quand le masque ou le calque disparaît.
- Affichage : le mode Masque montre la valeur du masque, et blanc sans masque.

## Risques

- **Précision** : un levels avec un gamma extrême ou une plage d'entrée étroite amplifie l'arrondi f16. La référence arrondit après chaque effet. Dans les piles aléatoires, gamma est tiré dans [0,5 ; 2] et `in_high − in_low` vaut au moins 0,2. Chaque effet reste testé seul sur toute sa plage.
- **Mémoire** : une texture 4K `R16Float` (32 Mo, plus les mips) par calque masqué. Elles sont jetables et comptées dans le pied du panneau Texture sets.
- **Interface** : la sélection par partie touche tous les usages de la sélection (supprimer, grouper, Propriétés, options de capture).

## Checklist de tests manuels

- [ ] Clic droit sur un calque : ajouter un masque noir (le calque disparaît), puis un masque blanc sur un autre calque (rien ne change).
- [ ] Ajouter un fill de masque, régler sa valeur, son mode et son opacité : le rendu suit en direct, avec une entrée d'historique par geste.
- [ ] Ajouter un levels et une inversion ; réordonner les effets par glisser-déposer ; masquer un effet avec l'œil.
- [ ] Touche M : le masque du calque sélectionné s'affiche en niveaux de gris dans la vue 3D et la vue 2D ; à nouveau M pour revenir.
- [ ] Masque sur un dossier : tout le groupe est masqué.
- [ ] Supprimer un effet, puis le masque (Suppr) ; Ctrl+Z et Ctrl+Y sur chaque opération.
- [ ] Enregistrer, rouvrir : masques identiques.
- [ ] `cargo run --release -p patine_cli -- bench compositor --masks` sur la RTX A2000.

## Mesures (11/10/2026, RTX 3080 Ti, build release)

`cargo run --release -p patine_cli -- bench compositor [--masks]` : sphère, un texture set à 5 canaux en 4096², 30 fills aux modes variés à 70 %. Avec `--masks`, chaque fill a un masque blanc, un fill de masque à 80 % en Produit et un levels (gamma 1,5). On mesure 20 modifications du fill du bas, attente GPU et mips comprises.

| Pile 30 fills, 4096² | Sans masques | Avec masques |
| --- | --- | --- |
| Première évaluation | 56 ms | 138 ms |
| Valeur d'un canal du fill du bas | 8,1 ms | 8,2 ms |
| Visibilité du fill du bas (5 canaux) | 33,1 ms | 34,6 ms |
| Fill du masque du fill du bas (5 canaux) | — | 36,1 ms |
| Mémoire du pool | 828 Mo | 2 170 Mo, dont 1 342 Mo de masques |

Le budget « paramètre d'un fill » du M5 (100 ms) tient avec les masques. Modifier un masque coûte sa réévaluation (deux passes `R16Float`), puis les mélanges des 5 canaux à partir de son calque.

La mémoire est le point à suivre. Chaque masque 4K pèse 45 Mo avec ses mips, et 30 calques masqués ajoutent 1,3 Go. Le budget de PLAN.md (6 Go sur la RTX 3080 Ti, 3,5 Go sur la RTX A2000) tient pour l'instant, mais ce sera à remesurer avec les contenus du M7 et du M8. Piste, à évaluer seulement si la mesure l'exige : garder les masques jetables hors du calque édité, puisqu'ils sont déjà recalculables.

## Écarts constatés pendant l'implémentation

- **Sélection** : `LayersState` garde `selected: Option<LayerId>` et gagne `part: LayerPart` (`Content`, `Mask` ou `MaskEffect(id)`). Le plan prévoyait une structure `Selection` ; le champ séparé change moins de code.
- **Fills de masque** : ils passent par la passe `mask_effect.wgsl`, comme Levels et Invert, et non par la passe de mélange du M5. Seule cette passe borne le résultat à [0, 1], ce qu'exige la décision 1 (un fill en Addition dépasserait 1).
- **`GpuDocument::sync`** signale aussi les sets dont un masque a été libéré : la matière qui affichait ce masque doit perdre la vue d'une texture détruite.
- **Masque absent dans le mode Masque** : l'application lie une texture blanche de 1 × 1 (`white_mask`). Le libellé indique alors que le calque n'a pas de masque.
- **`SetMask`** n'accepte que des effets déjà présents dans le masque courant, afin que tout nouvel effet passe par `InsertMaskEffect`, qui alloue son identifiant. La validation n'a lieu qu'à la première application, pour qu'un second `apply` signale bien l'état incohérent.
- **Libellés d'historique** : l'effet ajouté ou modifié est nommé (fill, levels, inversion) par un sélecteur Fluent sur l'argument `effect`.
- **Bench** : l'option `--masks` mesure aussi une modification de masque, et la mémoire du pool est affichée.
- **Risque relevé hors du M6** : le pool détruit tout de suite une texture jetable évincée, même si elle sert dans l'encodeur pas encore soumis de `GpuDocument::sync`. Cela n'arrive qu'au-delà du budget du pool (8 Go dans l'application). Une tâche séparée est proposée pour différer la destruction après la soumission.

## État à la clôture (11/10/2026)

- Critère 1 : les 11 effets testés (fills, levels sur toute leur plage, inversion), dans les 25 modes et à 3 opacités, donnent sur le GPU la valeur de la référence à 1/512 près, sur WARP et sur la RTX 3080 Ti. Les piles aléatoires de calques et de dossiers masqués égalent `reference::composite_stored`, sans échec sur 30 exécutions de 24 piles tirées au hasard (WARP et RTX).
- Critère 2 : `SetMask`, `InsertMaskEffect`, `RemoveMaskEffect`, `MoveMaskEffect` et `SetMaskEffect` s'annulent et se rétablissent à l'identique (tests de commandes). Les proptests d'undo tirent des opérations de masque aléatoires. Côté GPU, annuler tout redonne la composition d'origine et libère les textures de masque.
- Cache : modifier un effet ne réévalue que ce masque (2 passes), puis 3 mélanges depuis le cache inférieur. Sans changement, aucune passe n'est faite.
- Fixture v1 relue sans changement de format.
- 223 tests passent sur WARP.
- **À vérifier à la main** : la checklist ci-dessus, et le bench sur la RTX A2000.
