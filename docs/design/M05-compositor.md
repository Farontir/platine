# M5 — Compositeur v1

Note de conception du milestone M5. Objectif : une pile de calques fill complète et rapide.

Critères d'acceptation :
1. chaque blend mode correspond à la référence CPU à 1/512 près ;
2. le budget « paramètre d'un fill » est tenu : changer un paramètre d'un fill en bas d'une pile de 30 calques 4K prend moins de 100 ms sur la RTX 3080 Ti, moins de 250 ms sur la RTX A2000.

## Décisions

1. **Passes de rendu plutôt que compute.** Chaque mélange est une passe plein écran qui lit la composition courante (texture A) et écrit la suivante (texture B), en alternance (*ping-pong*). Cela marche pour `Rgba16Float` et `R16Float` sans la feature de stockage `R16Float`, et les tuiles sales deviennent des rectangles de *scissor*. Un fill uniforme est une constante passée en uniform : aucune texture de contenu au M5 (le cache de contenu viendra avec les procéduraux et la peinture).
2. **Formule.** Pour chaque canal, en partant de la valeur par défaut, chaque calque visible applique `dst = mix(dst, blend(dst, C), M × α)`, avec C le contenu, M le masque et α l'opacité du canal. Le masque vaut 1 jusqu'au M6 ; la passe lit déjà une texture de masque (1×1 blanche).
3. **Dossiers.** Un dossier évalue ses enfants dans une texture temporaire initialisée avec la composition du dessous, puis se mélange comme un seul calque, avec son mode et son opacité par canal : en mode Normal, l'opacité du dossier atténue tout le groupe, et un dossier à 100 % équivaut à ses enfants posés directement. Le mode **Passthrough** applique les enfants directement dans la pile parente. Sans couverture alpha par pixel (elle arrive avec la peinture au M8), c'est la définition la plus simple qui reste cohérente ; le M8 pourra la préciser sans changer le format.
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
