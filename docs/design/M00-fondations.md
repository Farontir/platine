# M0 — Fondations

Note de conception du milestone M0. Objectif : une application qui s'ouvre, une CI verte et la structure du dépôt en place.

## Toolchain et versions

- Rust figé par `rust-toolchain.toml` (1.99, avec rustfmt et clippy). `rust-version = "1.95"` dans le workspace : c'est la MSRV imposée par egui 0.36.
- Versions vérifiées sur crates.io le 10/10/2026 et déclarées dans `[workspace.dependencies]` :
  wgpu/naga 30.0.1, egui/egui-wgpu/egui-winit 0.36.2, egui_dock 0.21.1, egui-phosphor 0.14, winit 0.30.13 (la 0.31 est en bêta et egui-winit exige 0.30), glam 0.34, gltf 1.4, tobj 4.0, ufbx 0.11, image 0.25, exr 1.74, rayon 1.12, serde 1.0, ron 0.12, postcard 1.1, zstd 0.14, zip 9.0, rfd 0.17, thiserror 2.0, anyhow 1.0, tracing 0.1, tracing-subscriber 0.3, puffin 0.20, puffin_http 0.17, proptest 1.11, criterion 0.8, fluent-bundle 0.16, unic-langid 0.9, pollster 1.0, bytemuck 1.25, octotablet 0.1, bevy_mikktspace 1.0.
- Une crate n'ajoute une dépendance externe qu'à son premier usage : `Cargo.lock` ne fige donc que ce qui sert réellement.

## Écarts par rapport au plan initial

- **bincode → postcard.** bincode 3.0.0 est une version « tombstone » (`compile_error!`) et la 2.x est déclarée non maintenue. postcard est basé sur serde, avec un format binaire stable et spécifié. Concerne M3.
- **puffin_egui → puffin_http.** puffin_egui 0.30 dépend d'egui 0.33 ; mélanger deux versions d'egui est impossible dans la même UI. Le profileur s'active depuis l'application, qui démarre un serveur `puffin_http` ; on l'observe avec `puffin_viewer`. À réévaluer quand puffin_egui suivra egui.
- **Licences.** En plus de MIT, Apache-2.0, BSD et zlib, `deny.toml` autorise Unicode-3.0 (tables Unicode des crates ICU) et ce que le graphe réel exige, chaque ajout étant commenté. Les polices par défaut d'egui (OFL-1.1, Ubuntu Font Licence) sont tolérées par une exception limitée à `epaint_default_fonts`. Ces licences permettent la redistribution commerciale ; une police maison pourra les remplacer plus tard.

## Structure

- Workspace de 12 crates (`crates/patine_*`), conforme à `docs/PLAN.md`. Graphe interne :
  - `patine_core` : aucune dépendance GPU ni UI.
  - `patine_mesh` → core.
  - `patine_gpu` → core.
  - `patine_render`, `patine_compositor`, `patine_procedural`, `patine_paint`, `patine_bake` → core, gpu (+ mesh selon le besoin).
  - `patine_export`, `patine_assets` → core, gpu.
  - `patine_app` → tout ; `patine_cli` → tout sauf l'UI.
- La règle « pas de wgpu/egui dans `patine_core` » est vérifiée mécaniquement par `cargo deny` (`bans` avec `wrappers` : seules les crates listées peuvent dépendre directement de wgpu, egui et winit).
- Lints du workspace : `missing_docs`, `clippy::unwrap_used` (autorisé dans les tests via `clippy.toml`), `clippy::undocumented_unsafe_blocks`. Avec `-D warnings` en CI, les règles de CLAUDE.md deviennent bloquantes.

## Contexte GPU (`patine_gpu`)

- `GpuContext { instance, adapter, device, queue }`.
  - `GpuContext::new_headless(AdapterChoice)` pour les tests et le CLI.
  - `GpuContext::for_surface(...)` pour l'application.
- `AdapterChoice::Software` utilise `force_fallback_adapter` : WARP (DX12) sous Windows, lavapipe (Vulkan) sous Linux. `AdapterChoice::Hardware` prend l'adaptateur haute performance.
- Shaders : `patine_gpu::shaders::load("gpu/fill")` renvoie le source WGSL. En build normal, les fichiers sont embarqués par `include_str!` via une table générée à la compilation (`build.rs` qui parcourt `shaders/`). Avec la feature `hot-reload`, ils sont relus depuis le disque à chaque chargement. Point d'entrée unique pour ajouter plus tard un préprocesseur `#import` (WGSL n'a pas d'include ; nécessaire au M7).

## Outillage de test (`patine_gpu::testing`, feature `testing`)

- `gpu_test!(fn nom(ctx) { ... })` produit un `#[test]` qui crée un `GpuContext` headless.
  - **Par défaut, adaptateur logiciel**, en local comme en CI : les images de référence sont ainsi identiques partout.
  - `PATINE_TEST_ADAPTER=hardware` force la vraie carte, avec la tolérance « matériel ».
  - Aucun adaptateur disponible : échec explicite, jamais de saut silencieux.
- Comparateur d'images de référence : `assert_golden(crate, nom, &image, Tolerance)`.
  - Images RGBA8 en PNG dans `tests/golden/<crate>/<nom>.png`.
  - Échec : écrit `actual.png` et `diff.png` dans `target/golden-failures/<crate>/`.
  - `PATINE_UPDATE_GOLDEN=1` réécrit la référence.
  - La tolérance combine un écart maximal par canal et une proportion maximale de pixels hors tolérance.
- Validation des shaders : un test parse et valide tous les `shaders/**/*.wgsl` avec naga.

## Logs, profilage, console

- `tracing-subscriber` : filtre `RUST_LOG` (défaut `info`), sortie terminal, et une couche maison qui copie chaque événement dans un ring buffer partagé (2 000 lignes) lu par le panneau Console.
- Profileur : `puffin::set_scopes_on`, piloté par l'action « Profileur » ou `PATINE_PROFILE=1`. Le serveur `puffin_http` démarre à la première activation, sur le port par défaut de puffin.

## Traductions et actions

- `i18n/en/patine.ftl` et `i18n/fr/patine.ftl`, embarqués et chargés par `fluent-bundle`. La langue système est lue par `sys-locale`, avec repli sur l'anglais. Un menu permet de changer de langue à chaud.
- `Action { id, label_key, shortcut, enabled }` dans un registre unique. Les menus et la gestion des raccourcis lisent ce registre.
- Tests : mêmes clés dans les deux langues, chaque action a un libellé traduit, aucun raccourci en double.

## Application

- winit 0.30 (`ApplicationHandler`), surface wgpu, egui-winit + egui-wgpu.
- Disposition egui_dock :
  - Viewport 3D au centre, avec la Vue 2D en onglet.
  - À droite : Texture Sets, Calques, Propriétés, Affichage, Historique.
  - En bas : Bibliothèque et Console.
- Thème sombre, icônes Phosphor (MIT).
- Gestion du redimensionnement, de la minimisation et de la perte de surface (`SurfaceError::Lost/Outdated` → reconfiguration).

## Risques

- **R16Float en storage texture** : ce format n'est pas garanti par WebGPU de base, alors que le compositeur en a besoin pour les canaux scalaires. Un test M0 détecte le support sur l'adaptateur. En cas d'absence, repli sur R32Float ou Rgba16Float, à trancher au M4.
- **lavapipe en CI** : les versions de Mesa varient d'un runner à l'autre. Les tolérances des images de référence doivent absorber de petits écarts.
- **CI non vérifiable localement** : pas de remote GitHub au M0. Les jobs sont exécutés localement sous Windows ; le job Linux reste à confirmer après le premier push.

## Checklist de tests manuels

- [ ] `cargo run -p patine_app` ouvre une fenêtre sans erreur dans le terminal.
- [ ] Les 9 panneaux sont présents ; on peut les détacher, les déplacer et les réancrer.
- [ ] « Réinitialiser la disposition » restaure la disposition d'origine.
- [ ] La console affiche les logs de démarrage (adaptateur GPU, backend).
- [ ] Menu Langue : bascule FR ↔ EN de toute l'interface sans redémarrer.
- [ ] Activer le profileur, lancer `puffin_viewer` et voir les frames arriver.
- [ ] Redimensionner, minimiser puis restaurer la fenêtre : pas de crash, pas d'image figée.
- [ ] Ctrl+Q quitte l'application.
- [ ] Sur le portable RTX A2000 : mêmes vérifications.
