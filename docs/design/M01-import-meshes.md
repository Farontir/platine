# M1 — Import de meshes

Note de conception du milestone M1. Objectif : charger un mesh et en extraire tout ce dont les autres sous-systèmes ont besoin (viewport, peinture, polygon fill, baker).

Critères d'acceptation :
1. les trois formats (glTF, OBJ, FBX) chargent les fixtures ;
2. le nombre d'îlots UV est exact sur les meshes générés ;
3. un mesh sans UV est refusé avec un message clair.

## Choix des crates

- **glTF** : `gltf` 1.4. Les buffers sont chargés par `gltf::import_buffers`, sans décoder les images (inutiles à l'import du mesh).
- **FBX et OBJ** : `ufbx` 0.11 (bindings du C ufbx 0.23, compilé par `cc`). Écart par rapport au plan initial, qui prévoyait `tobj` pour l'OBJ :
  - un seul chemin de code pour les polygones ;
  - triangulation correcte des n-gones concaves (`triangulate_face`), alors que tobj ne triangule qu'en éventail ;
  - identifiants des polygones d'origine et matériaux `.mtl` ;
  - normalisation des axes et des unités ;
  - une dépendance de moins.
- **Tangentes** : `bevy_mikktspace` 1.0, mikktspace en Rust sûr, annoncé comme produisant des résultats identiques à l'implémentation de référence.

## Données

`patine_core::mesh::MeshAsset` est une donnée pure, sans GPU :
- attributs par sommet : `positions`, `normals`, `uv0` (optionnel), `tangents` (optionnel, présent si et seulement si `uv0`), `colors` (optionnel) ;
- `triangles` triés par matériau, avec `submeshes` (matériau → plage de triangles) ;
- par triangle : `triangle_polygon` (polygone d'origine) et `triangle_object` (objet d'origine) ;
- listes `objects` et `materials`.

Conventions : espace objet, Y vers le haut, repère droitier, mètres ; couleurs linéaires ; tangente `[x, y, z, signe]` avec bitangente = signe × cross(N, T), sans renormalisation après interpolation.

`MeshAsset::validate()` vérifie les invariants : longueurs, indices, valeurs finies, sous-meshes contigus et couvrants. Les tests et l'import l'appellent systématiquement.

Les UV sont optionnelles parce que le baker (M11) importera des high polys qui n'en ont pas. Un projet exige des UV : `ImportOptions::require_uvs` vaut vrai par défaut.

### Projet minimal

`patine_core::project` contient un sous-ensemble du modèle de M3 :
- `Project { mesh, mesh_source, texture_sets, normal_convention }` ;
- `TextureSet { id, name, material_index, resolution }`, un par matériau ;
- `Resolution` (puissance de 2, de 512 à 8192) ;
- `NormalConvention` (OpenGL ou DirectX).

Créer un projet remplace le document entier : ce n'est pas une `Command`, et l'historique repart de zéro. M3 ajoutera la pile de calques, les commandes et la sérialisation.

## Pipeline d'import

1. **Lecture** vers un `RawMesh` par coins de triangles (position, normale, UV, couleur, avec polygone, objet et matériau par triangle).
   - glTF : scène par défaut, transformations des nœuds, primitives Triangles, Strip et Fan.
   - FBX/OBJ : ufbx avec axes Y-up droitier et unités en mètres, triangulation face par face.
   - Une transformation de déterminant négatif inverse le winding. Les normales sont transformées par l'inverse transposée.
2. **Nettoyage** des triangles dégénérés, comptés dans le rapport.
3. **Normales manquantes** : plates pour glTF, comme l'exige la spécification ; lissées et pondérées par angle sur les positions soudées pour OBJ et FBX.
4. **Contrôle des UV** : `MeshError::MissingUvs { objects }` si elles sont exigées et absentes.
5. **Tangentes** mikktspace calculées sur les coins.
6. **Soudure** des coins identiques bit à bit, puis tri des triangles par matériau.
7. **Validation** et `ImportReport` : comptes, triangles retirés, avertissements, durées par étape.

Chaque variante de `MeshError` a une clé de traduction `error-mesh-*`. La correspondance est exhaustive et testée en français et en anglais.

## Topologie

- Les positions identiques bit à bit sont soudées pour définir les arêtes géométriques. Les arêtes non-manifold (plus de deux triangles) sont tolérées.
- **Îlot UV** : deux triangles partageant une arête géométrique sont connexes si les UV des deux extrémités de l'arête sont identiques des deux côtés.
- **Partie** : composante connexe de l'adjacence géométrique.
- Composantes calculées par union-find. L'adjacence est stockée en CSR.

## BVH

- SAH par bacs (16 bacs), feuilles de 4 triangles au plus, aplatissement en profondeur d'abord (l'enfant gauche suit son parent).
- Nœud `#[repr(C)]` de 32 octets `{ min, left_or_first, max, count }`, identique en mémoire côté CPU et GPU (std430) : le baker GPU (M12) l'enverra tel quel en storage buffer.
- Requêtes CPU : rayon le plus proche (barycentriques), rayon quelconque, boîte englobante.
- Construction séquentielle ; parallélisation seulement si les mesures l'exigent.

## Meshes de test

Générés par le code, avec des comptes connus :

| Générateur | Îlots UV | Parties |
| --- | --- | --- |
| `plane` | 1 | 1 |
| `cube` (une face par îlot) | 6 | 1 |
| `cube_cross` (dépliage en croix) | 1 | 1 |
| `uv_sphere` | 1 | 1 |
| `cylinder` (flanc et deux bouchons) | 3 | 1 |
| `two_cubes` | 12 | 2 |

`low_high_pair` fournit une sphère basse résolution et une sphère dense déplacée par une fonction analytique, pour le baker.

## Fixtures

`tests/fixtures/generate_fixtures.py` construit les scènes dans Blender par script, puis les exporte en glTF (`.glb` et `.gltf` + `.bin`), OBJ (+ `.mtl`) et FBX. Les fichiers générés sont commités sous licence CC0 (`tests/fixtures/LICENSE.md`).

Les scènes couvrent :
- plusieurs matériaux ;
- des n-gones concaves et des quads ;
- plusieurs objets avec transformations et échelle négative ;
- l'absence d'UV ;
- les couleurs de vertex.

Les tests vérifient que les trois formats d'une même scène donnent les mêmes comptes et la même boîte englobante.

## Application

- Action « Nouveau projet » (Ctrl+N) et dialogue modal :
  - fichier mesh avec bouton Parcourir (dialogue natif rfd, dans un thread de travail) ;
  - résolution ;
  - convention de normal map.
- L'import s'exécute dans un thread de travail ; l'interface affiche une progression puis, en cas d'erreur, le message traduit.
- Le panneau Texture Sets liste les texture sets. La Console affiche le rapport d'import.
- `patine_cli inspect <mesh>` affiche les statistiques sans aucune dépendance UI.

## Risques

- **Soudure bit à bit** : un exporteur peut écrire des positions presque égales de part et d'autre d'une couture. Si les fixtures le révèlent, repli sur une quantification relative à la boîte englobante.
- **Axes et unités FBX** : normalisés par ufbx, vérifiés sur la fixture multi-objets.
- **Mémoire** : environ 400 Mo de données intermédiaires pour un high poly de 2 M triangles. À mesurer.

## Checklist de tests manuels

- [ ] Ctrl+N ouvre le dialogue ; Parcourir ouvre le dialogue natif filtré sur `.glb`, `.gltf`, `.obj`, `.fbx`.
- [ ] Chaque fixture de `tests/fixtures/` se charge ; le panneau Texture Sets affiche un texture set par matériau.
- [ ] La fixture sans UV affiche un message clair, en français comme en anglais.
- [ ] Un FBX et un glTF personnels (exportés de Blender, Maya ou autre) se chargent, avec des comptes plausibles dans la Console.
- [ ] Un gros mesh (plus d'un million de triangles) se charge sans figer l'interface.
- [ ] `cargo run -p patine_cli -- inspect <fichier>` affiche des statistiques cohérentes avec la Console.

## Mesures (10/10/2026, RTX 3080 Ti, build release)

Sphère UV de Blender, 1024 × 1024 segments : 2 095 104 triangles, environ 4,2 M sommets (facettes plates), via `patine_cli inspect`.

| Étape | FBX | glTF (.glb) |
| --- | --- | --- |
| Lecture | 606 ms | 490 ms |
| Nettoyage | 19 ms | 23 ms |
| Normales | 37 ms | 37 ms |
| Tangentes (mikktspace) | 3 692 ms | 3 283 ms |
| Soudure et tri | 1 383 ms | 1 364 ms |
| **Import total** | **5,8 s** | **5,2 s** |
| Topologie | 1 361 ms | 1 292 ms |
| Construction du BVH | 1 691 ms | 1 909 ms (profondeur 25) |

Bench criterion (`cargo bench -p patine_mesh`) sur la sphère générée de 2,1 M triangles : 10 000 rayons en 25 ms.

Le plan ne fixe aucun budget pour l'import ; ces durées pèsent sur le baker (budget de 30 s pour un high poly de 2 M triangles) et sur la création d'un projet. Un low poly typique (moins de 200 000 triangles) s'importe en moins d'une demi-seconde, et l'import tourne dans un thread de travail, donc l'interface ne se fige pas. **Pas d'optimisation au M1.** Pistes mesurées, par ordre de gain :
1. Ne pas calculer les tangentes d'une source de bake : le baker n'en a pas besoin pour le high poly (−3,5 s).
2. Paralléliser mikktspace par partie connexe avec rayon (le résultat reste identique : mikktspace ne dépend pas de l'ordre des faces).
3. Clé de soudure plus compacte (hachage des 64 octets en deux passes) et tri parallèle.

## État à la clôture (10/10/2026)

- Les 6 tâches de M1 sont faites et testées : 25 tests unitaires et 6 tests d'import dans `patine_mesh`, plus 5 tests du dialogue dans `patine_app`.
- Critère 1 : les fixtures Blender se chargent dans les trois formats (`.glb`, `.gltf`, `.obj`, `.fbx`) et donnent les mêmes comptes, la même boîte englobante et la même aire.
- Critère 2 : le nombre d'îlots UV est exact sur tous les meshes générés.
- Critère 3 : la fixture sans UV est refusée (`MissingUvs`, avec le nom de l'objet) ; le message est traduit en français et en anglais.
- Défaut trouvé et corrigé par les tests : en f32, `sin(2π)` et `sin(π)` ne valent pas zéro, ce qui laissait ouverts la couture et le pôle nord des sphères générées.
- Le dialogue Nouveau projet est testé hors interface (import réel dans le thread de travail) ; son utilisation à la souris reste un test manuel.
