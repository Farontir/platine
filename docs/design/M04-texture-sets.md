# M4 — Texture sets et canaux sur GPU

Note de conception du milestone M4. Objectif : afficher les canaux d'un texture set et gérer la mémoire GPU.

Critères d'acceptation :
1. changer de résolution réalloue sans fuite, et la mémoire mesurée revient à la valeur attendue ;
2. la vue 2D affiche le bon wireframe UV.

## Écarts au plan

1. **Contenu des canaux.** Les piles de calques sont vides jusqu'au M5 : la texture finale d'un canal vaut sa valeur par défaut. `GpuDocument` alloue quand même la texture à la résolution du texture set, avec sa chaîne de mips, et la remplit par un *clear* de chaque niveau. Ce sont les cibles que le compositeur remplira. La génération de mips par downsample (avec sa référence CPU) arrive au M5, avec le premier contenu non uniforme.
2. **Éviction sans copie CPU.** Le pool compte la mémoire, applique un budget et détruit en priorité les textures *jetables* (caches recalculables) les moins récemment utilisées. Les textures finales du document sont *épinglées*. L'éviction vers la mémoire CPU compressée arrivera avec son premier client, les calques peints (M8).
3. **Facteurs de matériau.** Le shader multiplie les textures par des facteurs, comme glTF. Pour le document les facteurs valent 1. Sans textures de document (tests, bench, image de référence de la grille de sphères), le viewport lie des textures 1×1 blanches et place le matériau constant dans les facteurs : l'image de référence du M2 ne change pas.
4. **Plus de matériau d'aperçu** dans le panneau Paramètres d'affichage : les valeurs par défaut des canaux, dans le panneau Texture Sets, le remplacent. L'option `--material` des captures modifie ces valeurs par des commandes.
5. **Canaux éclairés.** Le shader éclairé lit la couleur de base, metallic, roughness, la normale (espace tangent mikktspace), l'AO (sur l'éclairage indirect seulement) et l'émissif. Height, opacity et les canaux User se voient en canal isolé ; la conversion de height en normale arrive au M9, la transparence plus tard.
6. **Pas de modèle de canaux dans le dialogue Nouveau projet** : on ajoute et retire les canaux dans le panneau Texture Sets.

## Document

Commande `SetChannelDef { set, channel, def: Option<ChannelDef> }`, motif *swap* :
- `Some` ajoute le canal à sa place canonique (ordre de `Channel`) ou remplace sa définition ;
- `None` le retire ;
- deux changements successifs du même canal fusionnent (glisser une couleur donne une entrée).

Retirer un canal ne touche pas aux calques : leurs réglages pour ce canal restent dans leur `ChannelMap` et resservent si le canal revient.

## Pool de textures (`patine_gpu::pool`)

- `TexturePool::new(budget)`, `allocate(descripteur, résidence) -> TextureId`, `get(id)`, `release(id)`, `memory()`.
- Taille comptée : octets par texel × surface, chaîne de mips comprise.
- Résidence : `Pinned` (jamais évincée) ou `Discardable`. Au-delà du budget, les `Discardable` les moins récemment utilisées (horloge logique incrémentée par `get`) sont détruites ; `get` renvoie alors `None` et le propriétaire recalcule.
- `release` détruit la texture tout de suite (`Texture::destroy`), sans attendre les autres références.
- Mesure indépendante : `Device::generate_allocator_report` donne la mémoire réellement allouée par wgpu sur DX12 et Vulkan.

## GpuDocument (`patine_compositor`)

Reflet GPU des texture sets : une texture par texture set et par canal actif, `Rgba16Float` pour les couleurs et normales, `R16Float` pour les scalaires, à la résolution du set, avec mips, épinglée.

`sync(ctx, pool, project) -> Vec<TextureSetId>` compare le document à ce qui est alloué :
- résolution ou format changé : la texture est libérée et réallouée ;
- canal retiré : libéré ; canal ajouté : alloué ;
- valeur par défaut changée : nouveau *clear*.

La fonction renvoie les texture sets modifiés, dont l'application refait les bind groups. Un undo emprunte le même chemin.

## Viewport (`patine_render`)

- `GpuMesh` garde les plages de triangles et d'arêtes de chaque sous-mesh ; les arêtes de polygones sont calculées par matériau, puis concaténées.
- Groupe de bind 2 (`GpuMaterial`) : couleur de base, metallic, roughness, normale, AO, émissif, un slot « canal inspecté », un sampler trilinéaire répété, et les facteurs.
- `Scene::materials: Option<&[GpuMaterial]>` indexé par matériau ; un draw par sous-mesh. Un canal absent d'un set est remplacé par une texture 1×1 neutre.
- Normal mapping : `n = normalize(T x + B y + N z)` avec `B = signe × cross(N, T)`, vecteurs interpolés non renormalisés (règle mikktspace).
- Modes d'affichage (`ViewSettings::display`) :
  - `Lit` ;
  - `Channel` : la valeur du canal inspecté, sans éclairage. Couleurs encodées en sRGB, données affichées telles quelles (0,5 = gris moyen), normales en `n × 0,5 + 0,5`. Exposition et tone mapping sont neutralisés ;
  - `UvChecker` : damier procédural sur les UV.

## Vue 2D (`UvView`)

Rendu hors écran, en espace UV, du canal affiché du texture set courant :
- panoramique et zoom (transformation UV → écran) ;
- damier neutre hors du carré 0–1, et contour du carré ;
- texture du canal (la couleur de base en mode éclairé), avec le même encodage que le mode canal ;
- wireframe UV : arêtes de polygones du sous-mesh, avec les UV comme positions. Les sommets sont séparés aux coutures UV, chaque sommet a donc un seul UV.

## Application

- Panneau Texture Sets : liste (clic = set courant), renommage, résolution, canaux du set courant (valeur par défaut, retrait, menu d'ajout), mémoire GPU en pied de panneau.
- Mode d'affichage dans Paramètres d'affichage ; F1 bascule entre éclairé et le dernier canal isolé, C passe au canal suivant ; le mode est rappelé dans la vue 3D.
- Vue 2D : molette pour zoomer, clic milieu (ou Alt + clic milieu) pour le panoramique, F pour cadrer quand la vue 2D a le survol.

## Tests

- `SetChannelDef` : apply/revert, fusion ; ajoutée aux tests de propriétés d'undo et de sauvegarde.
- Pool : comptage exact, éviction LRU des seules textures jetables, épinglées conservées.
- **Critère 1** : 2048 → 4096 → 2048, ajout et retrait d'un canal, undo de chaque commande. La mémoire du pool vaut la somme attendue à chaque étape et revient exactement à la valeur de départ ; le rapport d'allocation wgpu, quand il existe, aussi.
- Valeurs relues de chaque niveau de mip égales aux valeurs par défaut (f16).
- Image de référence de la grille de sphères inchangée ; textures constantes équivalentes aux facteurs ; normal map inclinée comparée à une référence CPU.
- Modes canal : valeur relue d'un scalaire, d'une couleur et d'une normale connus, à 1/255 près.
- **Critère 2** : wireframe UV du cube généré en 256². Les pixels des arêtes calculées sur le CPU sont tracés, l'intérieur des faces ne l'est pas, aucune diagonale n'apparaît ; plus une image de référence.

## Risques

- Mémoire : le casque (6 sets × 5 canaux) occupe environ 700 Mo en 2K et 2,8 Go en 4K avant tout calque. À surveiller sur la RTX A2000 ; l'allocation paresseuse (1×1 tant qu'un canal est uniforme) reste possible au M5 ou au M10.
- `generate_allocator_report` peut renvoyer `None` selon le backend : le test le signale et s'appuie sur le comptage du pool.
- Précision f16 des valeurs par défaut (environ 1e-3 relatif).

## Checklist de tests manuels

- [ ] Changer la résolution d'un texture set : la mémoire affichée suit ; Ctrl+Z la ramène à la valeur d'avant.
- [ ] Ajouter et retirer des canaux (AO, émissif, opacité, height, User), régler les valeurs par défaut : le rendu suit, une entrée d'historique par geste.
- [ ] F1 bascule entre éclairé et canal isolé ; C fait défiler les canaux du set courant ; le mode s'affiche dans la vue 3D.
- [ ] Damier UV sur le casque et la valise : pas d'étirement anormal.
- [ ] Vue 2D du casque et de la valise : wireframe UV conforme à celui de Blender ; panoramique, zoom, F.
- [ ] Enregistrer, rouvrir : canaux, valeurs par défaut et résolutions conservés.
- [ ] Rendu éclairé inchangé par rapport au M2 avec les valeurs par défaut équivalentes.
- [ ] `cargo run --release -p patine_cli -- bench viewport` sur la RTX A2000 : au moins 60 i/s avec les textures 4K.
