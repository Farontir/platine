# M3 — Document, commandes et sauvegarde

Note de conception du milestone M3. Objectif : un modèle de document fiable, entièrement annulable et sauvegardable.

Critères d'acceptation :
1. les tests de propriétés sur l'undo et la sauvegarde passent ;
2. un projet sauvegardé se rouvre à l'identique.

## Écarts au plan

1. **Un seul mesh par projet** (`Project::mesh`, pas `meshes: Vec`). Le mesh texturé est unique dans tout le MVP ; les high poly du bake (M11) seront des sources référencées par chemin dans les réglages de bake, pas des données du projet.
2. **Tous les types du modèle sont définis dès le M3**, sérialisables, même ceux dont la logique arrive plus tard (fill, paint, dossiers, instances, masques, effets, anchors, `StrokeLog`, baked maps, presets d'export). Les commandes de l'arbre de calques sont écrites et testées au M3 sans interface ; le panneau Calques arrive au M5.
3. **Environnement dans le document** : `EnvironmentSettings { source, rotation, exposure_ev }`. Une HDRI choisie par l'utilisateur est copiée dans le projet (`resources/`, nommée par son empreinte SHA-256), ce qui rend le fichier portable. Le tone mapping, le fond, les ombres, le fil de fer et la projection restent des préférences de vue.
4. **Migrations par types figés** plutôt que sur un arbre `ron::Value`, qui perd les noms d'enums et de structs.

## Modèle de données (`patine_core`)

- `ids` : `LayerId`, `EffectId`, `AnchorId` (u64) tirés d'un compteur unique `Project::next_id`, jamais réutilisé, sérialisé avec le projet. `TextureSetId` reste l'indice du matériau.
- `channel` : `Channel` (BaseColor, Metallic, Roughness, Normal, Height, AmbientOcclusion, Emissive, Opacity, User(u8)), `ChannelDef { channel, format, default }`, `ChannelMap<T>` = `BTreeMap<Channel, T>` (ordre stable dans le fichier). Modèle de canaux par défaut d'un nouveau texture set : couleur de base, metallic, roughness, normal, height.
- `layer` : `Layer { id, name, visible, kind, channels, mask, effects, anchor }`, `LayerKind` (Paint, Fill, Folder, Instance), `ChannelBlend { mode, opacity }`, `BlendMode` (liste du plan, plus Normal Map Combine et Normal Map Detail), `FillSource` (uniforme par canal, image référencée par empreinte, procédural `{ id, version, params }`), `Projection` (UV, triplanaire, planaire, sphérique, cylindrique, avec échelle, rotation et décalage).
- `mask` : `MaskStack { initial, effects }`, `MaskEffectEntry { id, visible, blend, opacity, effect }`, `MaskEffect` (Paint, Fill, Generator, Filter, Levels, AnchorRef, PolygonFill), `Effect` (filtre ou levels), `GeneratorKind`, `FilterKind`, `Params` = `BTreeMap<String, ParamValue>`.
- `stroke` : `StrokeLog { strokes }`, `Stroke { brush, tool, camera, symmetry, seed, dabs }`, `Dab { position, pressure, tilt, time }`. Les champs détaillés de la brosse seront complétés au M8 et M9, par migration.
- `environment`, `export`, `bake` : `EnvironmentSettings`, `ExportPreset` (forme RON du plan), `BakedMaps` (les 8 types de maps, vide jusqu'au M11).
- `project` : `Project { mesh: Arc<MeshAsset>, mesh_source, texture_sets, environment, export_presets, resources, normal_convention, next_id }`. `Project::validate()` vérifie l'unicité des identifiants, `next_id` supérieur à tous, l'existence des sources d'instance, des anchors et des ressources référencés.

Les données lourdes sont partagées (`Arc<MeshAsset>`, `Arc<[u8]>` pour les ressources, `Arc<StrokeLog>` modifié en copy-on-write) : cloner le projet est bon marché, et l'enregistrement travaille sur un instantané en arrière-plan.

## Commandes et historique

```rust
pub trait Command: Debug + Send + 'static {
    fn apply(&mut self, project: &mut Project) -> Result<(), CommandError>;
    fn revert(&mut self, project: &mut Project) -> Result<(), CommandError>;
    fn label(&self) -> Label;                       // clé de traduction + arguments
    fn merge(&mut self, next: &dyn Command) -> bool; // fusion des gestes continus
    fn heap_size(&self) -> usize;                   // budget mémoire
    fn as_any(&self) -> &dyn Any;
}
```

- `apply` laisse le document inchangé s'il échoue (commande construite sur un état périmé) : rien n'est empilé.
- `History` : piles undo et redo, `push`, `undo`, `redo`, `go_to(index)`.
- **Fusion** : une commande fusionne avec la précédente si `merge` l'accepte (même cible) et si l'historique n'a pas été scellé entre les deux. L'app scelle à la fin d'un geste (relâchement d'un slider ou d'un glisser) : un geste produit une seule entrée.
- **Budget** : 2 Go par défaut, réglable ; les entrées les plus anciennes sont évincées. `heap_size` est une estimation.
- **Marqueur enregistré** : chaque entrée a un numéro unique ; le document est modifié si l'entrée au sommet n'est pas celle de la dernière sauvegarde.
- Les libellés sont des clés de traduction : `patine_core` ne dépend pas de fluent.

Commandes du M3 : texture set (renommer, résolution), environnement (fusionnable), calques (insérer, supprimer, déplacer, renommer, visibilité, blend et opacité d'un canal). Les calques sont désignés par leur `LayerId` ; leur position est un `(parent, index)`.

## Format `.patine`

Archive zip. Les entrées sont stockées sans recompression : les binaires sont déjà compressés en zstd, et le manifeste reste lisible avec un simple `unzip`.

| Entrée | Contenu |
| --- | --- |
| `manifest.ron` | `format_version`, version de l'app, document sans pixels |
| `meshes/mesh.bin` | `MeshAsset` traité, postcard + zstd : la réouverture ne réimporte pas |
| `strokes/<id>.bin` | `StrokeLog` de chaque calque ou effet paint, postcard + zstd |
| `resources/<sha256>` | images importées par l'utilisateur (HDRI au M3) |
| `thumb.png` | vignette du viewport, 256 px de large |
| `cache/`, `baked/` | réservés au M10 et au M11 |

- Écriture atomique : fichier temporaire dans le même dossier, puis renommage.
- Lecture : une version plus récente que l'application est refusée avec un message clair ; archive corrompue et entrée manquante donnent des erreurs typées ; `validate()` s'exécute après la lecture.

### Versions et migrations

- `format_version` vaut 1 au M3.
- Le lecteur lit d'abord la version seule, puis désérialise les types de cette version et applique la chaîne `migrate_vN_to_vN1` jusqu'à la version courante.
- **Règle pour les milestones suivants** : toute modification du format sérialisé incrémente `format_version`, copie les types de l'ancienne version dans `file::vN` (figés), écrit la migration et ajoute une fixture `tests/fixtures/projects/vN_*.patine`, rechargée en test.

## Application

- `Document { project, history, path }` remplace `Option<Project>`. Nouveau projet, Ouvrir et Récupérer remplacent le document en bloc ; ce ne sont pas des commandes.
- L'interface ne modifie plus le document : les panneaux renvoient des commandes et des demandes de scellement dans `UiOutput`.
- Actions : Annuler (Ctrl+Z), Rétablir (Ctrl+Y), Ouvrir (Ctrl+O), Enregistrer (Ctrl+S), Enregistrer sous (Ctrl+Maj+S). Pendant la saisie de texte, Ctrl+Z et Ctrl+Y restent au champ.
- Environnement : le panneau Paramètres d'affichage et Maj + clic droit émettent des commandes. Le GPU suit le document : l'app compare la source du document à celle chargée et recharge si elles diffèrent, ce qui couvre l'undo. Sans projet, les réglages s'appliquent à un environnement « brouillon », repris par le prochain projet.
- Enregistrement et ouverture en arrière-plan ; le titre de la fenêtre porte le nom du projet et `*` s'il est modifié ; confirmation avant de perdre des modifications (Nouveau, Ouvrir, Quitter, fermeture).
- `patine_app projet.patine` ouvre un projet.

### Autosave et récupération

- Toutes les 5 minutes si le document a changé depuis le dernier autosave (`--autosave-interval SECS` pour les tests manuels), en arrière-plan.
- Dossier de session : `%APPDATA%\Patine\autosave\<session>\` (Linux : `$XDG_DATA_HOME/patine` ou `~/.local/share/patine`), avec `project.patine`, `info.ron` (chemin d'origine, date) et `session.lock`.
- La session garde un verrou exclusif sur `session.lock` (`File::try_lock`, std). Une sortie propre supprime le dossier.
- Au démarrage, un dossier dont le verrou est libre vient d'une session terminée par un crash : dialogue « Récupérer / Ignorer ». Le projet récupéré est marqué modifié et garde son chemin d'origine. Deux instances simultanées ne se gênent pas.

### Panneau Historique

Liste des entrées traduites, l'entrée courante en surbrillance et les entrées annulées grisées ; un clic revient à cet état. La première ligne, « Ouverture du projet », revient à l'état initial.

## Dépendances

`serde`, `ron`, `postcard`, `zstd`, `zip` (sans features : entrées stockées) et `proptest` sont déjà prévus au workspace (M0). Nouvelle : `sha2` (RustCrypto, MIT ou Apache-2.0), pour nommer les ressources par une empreinte stable entre versions de Rust.

## Tests

- Apply et revert de chaque commande ; fusion et scellement ; budget et éviction ; marqueur enregistré.
- **proptest** : une séquence aléatoire de commandes, puis tout annuler, redonne le document initial ; tout rétablir redonne le document final.
- **proptest** : enregistrer puis relire redonne un document identique (flottants au bit près).
- Fixture v1 relue ; version future refusée ; archive tronquée ou entrée manquante refusées proprement.
- Verrou de session : un dossier verrouillé n'est pas orphelin, un dossier libre l'est.

## Risques

- Taille et durée d'enregistrement des gros meshes (1 M triangles ≈ 70 Mo bruts) : zstd niveau 3 sur un thread, mesuré avec `patine_cli bench project`.
- Flottants dans le RON : vérifier que ron relit les f32 au bit près, sinon passer ces valeurs en binaire.
- Comptage mémoire de l'historique approximatif ; les tuiles de strokes (M8) seront le vrai poste.

## Checklist de tests manuels

- [ ] Modifier la rotation, l'exposition et l'HDRI, renommer un texture set : chaque geste donne une seule entrée dans l'Historique.
- [ ] Ctrl+Z et Ctrl+Y annulent et rétablissent, rendu compris ; un clic dans l'Historique revient à cet état.
- [ ] Ctrl+Z pendant le renommage d'un texture set annule la saisie, pas le document.
- [ ] Enregistrer, fermer, rouvrir : mesh, texture sets et environnement identiques ; le titre perd son `*` après l'enregistrement.
- [ ] Charger une HDRI personnelle, enregistrer, déplacer le `.patine` et le fichier HDRI d'origine, rouvrir : l'HDRI est restituée.
- [ ] Quitter, fermer la fenêtre, ouvrir ou créer un projet avec des modifications : la confirmation s'affiche, et chaque choix fait ce qu'il dit.
- [ ] Lancer avec `--autosave-interval 10`, modifier, attendre, tuer le processus dans le Gestionnaire des tâches, relancer : la récupération est proposée et rend le travail.
- [ ] Deux instances ouvertes en même temps : aucune ne propose de récupérer l'autre.
- [ ] `patine_app projet.patine` ouvre le projet.
- [ ] Ouvrir un fichier qui n'est pas un projet : message d'erreur clair.
