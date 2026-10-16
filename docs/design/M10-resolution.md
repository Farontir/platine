# M10 — Indépendance de résolution

Note de conception du milestone M10. Objectif : changer de résolution sans perte de qualité.

Critères d'acceptation :
1. peindre en 1K, passer en 4K puis réduire en 1K redonne l'original à la tolérance près ;
2. le résultat en 4K est net, pas agrandi.

Budget concerné : ouvrir un projet 4K de 30 calques en moins de 5 s (RTX 3080 Ti) et en moins de 10 s (RTX A2000).

## État de départ

- **Changement de résolution** : `SetTextureSetResolution` change la résolution d'un texture set, avec undo, depuis le panneau Texture sets.
- **Rejeu** : `PaintRasters::sync_set` relâche un raster dont la taille a changé, puis rejoue tout son `StrokeLog`.
- **Réévaluation** : fills, procéduraux, masques et normale d'ombrage sont réévalués à la nouvelle taille.
- **Ce qui rend la peinture indépendante de la résolution** :
  - une brosse est définie en pixels d'écran ;
  - la projection part du point de surface de chaque texel ;
  - le flou du bord est en pixels d'écran.

  Un texel 4K échantillonne donc la même fonction plus finement : le résultat est net par construction.
- **Problème** : `Painter::replay_stroke` rastérise tous les dabs d'un trait en un seul dispatch, sur toutes ses tuiles. Le coût est O(tuiles × dabs) : un long trait en 8K peut dépasser le timeout d'environ 2 s du pilote Windows. Cela enfreint la règle « dispatches longs découpés ».

## Décisions

### 1. Rejeu par lots

- `replay_stroke` découpe le trait en lots de `REPLAY_BATCH` dabs (64), chacun avec ses propres tuiles (`stroke_tiles` sur le lot), comme la peinture en direct.
- `rasterize_dabs` découpe aussi la liste de tuiles en dispatches d'au plus `MAX_TILES_PER_DISPATCH` tuiles (256).
- Le résultat ne change pas : l'accumulation d'un texel suit l'ordre des dabs, quel que soit le découpage. Le M8 et le M9 l'ont montré (direct par lots de 1 à 17 dabs = rejeu, à l'octet près), et un test dédié le vérifie.

### 2. Critères en test

Scène :
- un cube en gros plan, pour qu'un texel 1K couvre plusieurs pixels d'écran ;
- un fill en dessous ;
- un calque de peinture avec un trait à brosse dure et un trait avec alpha et jitters ;
- un effet de peinture de masque.

Vérifications :
- **Critère 1** : les canaux finaux en 1K (A), puis 4K (B), puis 1K (C). Attendu : C = A à 1/512 près.
- **Critère 2** :
  - le long du bord dur, on mesure la part des texels en transition (couverture entre 0,1 et 0,9), dans l'espace UV ;
  - en 4K, elle reste sous 0,4 × celle de l'agrandissement bilinéaire de A ;
  - un témoin, l'agrandissement lui-même, échoue au même seuil.
- **Exactitude** : B est égal à la même peinture faite en direct en 4K.
- **Undo** : annuler un trait après l'aller-retour rejoue, sans utiliser d'instantané périmé.

### 3. Résolution d'export

- **`patine_compositor::offline::evaluate_set(ctx, project, set, resolution)`** :
  - une copie du projet réduite à ce texture set, à la résolution demandée ;
  - un `GpuDocument` et un pool propres, une synchronisation, puis la relecture de chaque canal final en `f32` ;
  - tout est relâché à la fin ; le document de travail n'est pas touché.
- **`patine_cli channels projet.patine --out DOSSIER [--resolution N] [--set NOM]`** :
  - un PNG 16 bits par canal ;
  - couleur de base et émissif en sRGB, les autres canaux linéaires ;
  - normale encodée × 0,5 + 0,5.

  C'est un avant-goût de l'exporteur du M17, qui reprendra `evaluate_set`.
- **Document** : pas de champ de résolution d'export. Ce sera un réglage de l'export (M17).

### 4. Caches raster

Le cache est une donnée dérivée : le document ne change pas, `FORMAT_VERSION` non plus. Sans cache, ou si le cache est périmé ou illisible, on rejoue. Les versions actuelles ignorent les entrées qu'elles ne connaissent pas.

**Format**
- Une entrée par nœud peint : `cache/<id>.bin` (postcard + zstd), où l'id est celui du calque ou de l'effet de masque, comme pour `strokes/`.
- Contenu : une clé, la taille, et, pour chaque canal, la disposition, les tuiles peintes et leurs texels f16 bruts.
- **Écart au plan** (« caches raster EXR ») : un EXR plein en 4K ferait 128 Mo par canal à relire et à écrire, et l'EXR tuilé ne permet pas d'omettre des tuiles. Les tuiles peintes seules, en f16 exact et compressées par zstd, restent petites et se relisent vite.

**Clé** : SHA-256 de :
- `CACHE_VERSION` : la version de l'application, plus un compteur à incrémenter à chaque changement de la rastérisation ou des shaders procéduraux ;
- la résolution et l'index de matériau ;
- la liste des canaux ;
- `StrokeLog::fingerprint()`, le SHA-256 du postcard du log.

Le mesh n'entre pas dans la clé : il est écrit dans la même archive, au même instant que les caches. Un futur ré-import de mesh devra jeter les caches.

**Tuiles peintes** : chaque `Raster` tient l'union des tuiles touchées par ses traits, en direct comme au rejeu. L'undo garde l'ensemble, ce qui reste conservateur ; un rejeu complet le recalcule.

**Enregistrement**
- `GpuDocument::raster_caches` relit seulement les tuiles peintes, sur le thread principal, une copie par tuile dans un tampon de relecture.
- L'encodage (zstd) se fait dans le thread d'enregistrement.
- Un trait en cours n'est pas inclus.

**Ouverture**
- Le thread d'ouverture décode les caches (`RasterCache::decode`).
- `GpuDocument::provide_caches` les garde jusqu'à la synchronisation.
- Une cible vide dont la clé correspond reçoit ses tuiles (`write_texture`) et adopte le log, sans rejeu.

**Application**
- « Enregistrer » joint les caches. La bascule Fichier › « Enregistrer les caches de peinture », activée par défaut, passe par le registre `Action`.
- L'autosave n'écrit pas de caches : il tourne dans un thread, sans GPU.
- La console indique les cibles chargées depuis le cache et celles rejouées.

### 5. Mesure

- **`patine_cli bench resolution`** : un projet avec N traits sur la sphère. On mesure le rejeu en 1K, 4K, 8K, puis de retour en 1K, ainsi que la pire durée d'un lot, soumis et attendu seul.
- **`patine_cli bench open --layers 30 --resolution 4096 [--no-caches]`** :
  - les temps d'enregistrement, puis d'ouverture et de synchronisation, comparés au budget de 5 s ;
  - la taille du fichier ;
  - la mémoire.

## Structures

```rust
// patine_paint::raster
pub const REPLAY_BATCH: usize = 64;
pub const MAX_TILES_PER_DISPATCH: usize = 256;

// patine_core::stroke
impl StrokeLog { pub fn fingerprint(&self) -> ResourceHash }

// patine_core::file
pub fn write(project, writer, thumbnail, caches: &[(u64, Vec<u8>)]) -> Result<W, FileError>;
pub fn read_with_caches(reader) -> Result<(Project, BTreeMap<u64, Vec<u8>>), FileError>;

// patine_compositor::cache
pub struct RasterCache { key: [u8; 32], size: u32, channels: Vec<CachedChannel> }
pub struct CachedChannel { channel: Channel, layout: ContentLayout, tiles: Vec<[u32; 2]>, texels: Vec<u8> }

// patine_compositor::offline
pub struct ChannelImage { def: ChannelDef, size: u32, texels: Vec<[f32; 4]> }
pub fn evaluate_set(ctx, project, set: TextureSetId, resolution: Resolution) -> Result<Vec<ChannelImage>, OfflineError>;
```

## Tests

- **Rejeu par lots** : égal au rejeu en un bloc, à l'octet près, avec alpha, jitters et symétrie. Direct = rejeu (tests existants).
- **Critères 1 et 2**, comme décrit plus haut, plus la peinture directe en 4K et l'undo après l'aller-retour.
- **`evaluate_set`** :
  - égal au document de travail à la même résolution ;
  - en 2K, égal à un document en 2K ;
  - document de travail intact (textures, mémoire, compteur de rejeux).
- **`patine_cli channels`** : fichiers écrits, taille, quelques pixels.
- **Core** : caches en aller-retour, fichier sans cache, cache illisible ignoré, empreinte stable et sensible au log.
- **Caches** :
  - ouverture avec caches : zéro rejeu et contenus identiques à l'octet près ;
  - clé périmée (résolution, version, log) : rejeu ;
  - cache corrompu : rejeu ;
  - masques peints compris.

## Risques

- **Mémoire en 8K** : un canal `Rgba16Float` fait 512 Mo, et les contenus peints sont épinglés. Le bench le mesure ; en cas de dépassement, on rapporte, sans contourner.
- **Temps d'enregistrement avec caches** : la relecture se fait sur le thread principal. Seules les tuiles peintes sont relues, et le temps est mesuré.
- **Cache périmé appliqué à tort** : la clé couvre le log, la résolution, les canaux et la version. Des tests forcent chaque invalidation.

## Checklist de tests manuels

1. Peindre plusieurs traits en 1K (brosse dure, alpha, jitters), passer le texture set en 4K : la peinture est nette en gros plan, pas floue.
2. Revenir en 1K : identique à l'état de départ. Ctrl+Z (retour en 4K), puis Ctrl+Z sur un trait.
3. 8K sur la valise : pas de gel au-delà de quelques secondes, pas de perte du périphérique.
4. Enregistrer avec caches, rouvrir : ouverture rapide, la console indique les caches utilisés.
5. Désactiver « Enregistrer les caches de peinture », enregistrer : fichier plus petit ; rouvrir : rejeu, même résultat.
6. Ouvrir un projet du M9 (sans caches) : rejeu, identique.
7. `patine_cli channels manual_tests/casque.patine --out sortie --resolution 2048` : un PNG par canal, lisible.
8. `patine_cli bench resolution` et `bench open --layers 30 --resolution 4096` sur la RTX A2000.

## Mesures (16/10/2026, RTX 3080 Ti, build release)

**Changement de résolution** : `bench resolution`, une sphère avec 4 calques peints de 25 traits de 400 dabs (60 px).

| Résolution | Rejeu et composition des 100 traits | Pool |
| --- | --- | --- |
| 1024² | 162 ms | 117 Mo |
| 4096² | 328 ms | 1,9 Go |
| 8192² | 950 ms | 7,5 Go |
| retour en 1024² | 180 ms | 117 Mo |

**Pire trait rejoué seul en 8K** : 52 ms, loin du timeout du pilote (environ 2 s).

**Ouverture** : `bench open`, 4K, 30 calques dont 10 peints (traits de 300 dabs), jusqu'à la première évaluation complète.

| Traits par calque peint | Sans caches | Avec caches | Taille des caches |
| --- | --- | --- | --- |
| 20 (200 traits) | 0,77 s | 1,37 s | 89 Mo |
| 50 (500 traits) | 1,27 s | 0,96 s | 83 Mo |
| 200 (2 000 traits) | **5,6 s** (budget dépassé) | **0,95 s** | 57 Mo |

- **Lecture** : avec caches, le temps d'ouverture ne dépend plus du nombre de traits, mais de la surface peinte (environ 1 Go de texels bruts ici, décompressés en parallèle).
- **Budget** : il est dépassé sans caches pour un projet longuement peint, et tenu avec caches.
- **Enregistrement** : la relecture des tuiles sur le thread principal prend 0,42 à 0,48 s ; l'encodage, en parallèle, 0,26 à 0,37 s.
- **Projet peu peint** : rejouer reste alors plus rapide que lire les caches. Le gain dépend du projet.

**Peinture en direct** : pas de régression. Dab de 80 px : 2,4 ms en moyenne ; avec 16 copies : 8,2 ms en moyenne, contre 7,9 ms au M9, ce qui reste dans la variance de la mesure.

**Export** : `patine_cli channels` en 2048² sur le casque. Pour chacun des 6 texture sets, environ 135 ms d'évaluation et 235 ms d'écriture des 5 PNG.

## Écarts constatés pendant l'implémentation

Les mesures ont révélé trois défauts du M8, que le 8K rendait visibles. Chacun est corrigé, mesuré et testé.

- **Tampons de trait en attente.**
  - *Défaut* : un rejeu de N traits dans un même encodeur gardait N tampons de trait (256 Mo chacun en 8K) jusqu'à la fin de la synchronisation, sans que le pool les compte.
  - *Correction* : `TexturePool::allocate_recycled` redonne une texture relâchée de même description, l'ordre des commandes garantissant que l'ancien usage précède le nouveau. Le rejeu de 100 traits en 4K est passé de 1 474 à 445 ms, et le 8K ne manque plus de mémoire.
- **Images de profondeur.**
  - *Défaut* : créées directement sur le périphérique (environ 16 Mo chacune), elles s'accumulaient de la même façon. 2 000 traits en 4K manquaient de mémoire.
  - *Correction* : elles passent maintenant par le pool, avec la même règle de recyclage.
- **Textures de travail et pic de mémoire.**
  - *Défaut* : les textures de travail du compositeur restaient allouées pour chaque taille déjà utilisée. Après un passage par la 8K, 1,8 Go restait vivant de retour en 1K. Et un texture set redimensionné gardait son ancienne taille jusqu'à la fin de la synchronisation, si bien que le pic était la somme des deux tailles.
  - *Correction* : au début de `sync`, tout ce qu'il tient est libéré et détruit, puis les tailles qui ne servent plus sont lâchées. Le test d'aller-retour vérifie que le pool revient exactement à la mémoire d'un document ouvert en 1K ; sans la correction, il échoue (130 Mo contre 47 Mo).
- **Format des caches.**
  - *Choix* : tuiles peintes en f16 exactes, compressées par zstd, au lieu de l'EXR du plan (voir les décisions).
  - *Organisation* : un en-tête postcard, puis les texels bruts. Désérialiser les texels avec postcard les lisait octet par octet : 1,8 s pour 89 Mo.
  - *Parallélisme* : l'encodage et le décodage tournent sur tous les cœurs.
- **Critère 2** : la netteté se mesure sur un trait dur, dans le masque. Le trait à jitters a un bord flou de plusieurs pixels d'écran, aussi large à toutes les résolutions : il ne dit rien de la netteté.
- **Ligne de commande** : `--edit resolution=N` règle la résolution du texture set courant, pour les captures.
- **Hors du M10**, comme prévu :
  - le moteur d'export et ses presets (M17), qui reprendra `offline::evaluate_set` ;
  - le déchargement en mémoire CPU des contenus peints épinglés. En 8K, un calque peint sur deux canaux occupe 768 Mo épinglés : avec 4 calques peints, le pool atteint 7,5 Go. Rien n'empêche encore de choisir le 8K sur un projet qui ne tiendrait pas en mémoire GPU : une estimation avant changement de résolution serait à prévoir.

## État à la clôture (16/10/2026)

- **Critère 1** : de retour en 1K après la 4K, les canaux finaux et les contenus peints sont identiques à l'original (écart maximal : 0), sur WARP (512 puis 2048) et sur la RTX 3080 Ti (1024 puis 4096). Le pool revient exactement à la mémoire et aux textures d'un document ouvert en 1K.
- **Critère 2** : le long du trait dur, la 4K a 296 texels en transition, contre 4 741 pour l'agrandissement bilinéaire de la 1K (RTX), soit 6 %. Un trait rejoué en 4K est égal à la même peinture faite en direct en 4K.
- **Rejeu par lots** : égal au rejeu en un bloc, à l'octet près (lots de 1, 7 et le trait entier ; une tuile par dispatch).
- **Export** : `evaluate_set` à la résolution de travail est égal au document ; à une autre résolution, il est égal à un document à cette résolution. Le document de travail n'est pas touché. `patine_cli channels` est testé de bout en bout.
- **Caches** :
  - ouverture sans rejeu, texels identiques à l'octet près ;
  - rejeu pour une résolution, une clé, un log ou des texels qui ne correspondent pas ;
  - octets abîmés refusés ;
  - dans l'application, l'ouverture d'un fichier avec caches donne 10 calques chargés et 0 trait rejoué.
- **Tests** : 337 passent sur WARP ; ceux de `patine_gpu`, `patine_paint` et `patine_compositor` passent aussi sur la RTX 3080 Ti.
- **À vérifier à la main** : la checklist ci-dessus, en particulier l'enregistrement avec caches depuis l'application (Ctrl+S, puis la bascule du menu Fichier), et les benchs sur la RTX A2000.
