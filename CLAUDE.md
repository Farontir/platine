# Patine — instructions pour Claude Code

Patine est un logiciel de texturing PBR par calques sur mesh 3D (équivalent fonctionnel de Substance Painter).
Le plan complet est dans `docs/PLAN.md` : c'est la source de vérité pour la stack, l'architecture et les milestones.

## Commandes
- Lancer : `cargo run -p patine_app`
- Lancer avec rechargement des shaders : `cargo run -p patine_app --features hot-reload`
- Tests : `cargo test --workspace`
- Lint : `cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings`
- Licences : `cargo deny check`
- Régénérer les images de référence : `PATINE_UPDATE_GOLDEN=1 cargo test -p <crate>`

## Méthode de travail
1. Avant un milestone, relis sa section dans `docs/PLAN.md` et les sections d'architecture qu'il touche.
2. Commence en plan mode. Écris `docs/design/Mxx-<nom>.md` : choix de conception, structures, risques, checklist de tests manuels.
3. Avance par petites tâches. Après chaque tâche : fmt, clippy, tests, puis un commit (Conventional Commits).
4. Coche la tâche dans `docs/PLAN.md` seulement quand ses tests passent.
5. Un milestone n'est terminé que si tous ses critères d'acceptation sont vérifiés. Termine par un résumé : fait, reste à faire, tests manuels à réaliser.
6. Après trois échecs sur le même problème, arrête-toi et écris un rapport (hypothèses testées, pistes) au lieu de contourner.

## Règles d'architecture
- `patine_core` ne dépend jamais de wgpu ni d'egui.
- Toute modification du document passe par une `Command` avec `apply` et `revert`. L'UI n'écrit jamais dans le document.
- Le GPU n'est qu'un cache : aucune donnée du document n'existe seulement en pixels, sauf les images importées.
- Une seule fonction rastérise les strokes (`rasterize_dabs`), pour la peinture en direct comme pour le rejeu.
- Chaque algorithme GPU a une implémentation CPU de référence et un test de comparaison.
- Shaders en WGSL dans `shaders/`, validés par naga en test. Aucun shader inline dans le code Rust.
- Les dispatches GPU longs sont découpés par tuiles (timeout du pilote Windows d'environ 2 s).
- Les actions de l'UI passent par le registre `Action`. Tous les textes passent par les fichiers de traduction.

## Code
- Rust stable, édition 2024. Code, identifiants et commentaires en anglais ; notes de conception en français.
- `thiserror` dans les crates, `anyhow` seulement dans les binaires.
- Pas de `unwrap()` hors des tests. `expect()` avec un message qui explique l'invariant.
- Pas de `unsafe` sans commentaire `// SAFETY:` qui le justifie.
- Aucune nouvelle dépendance sans justification dans le message de commit. Licences permissives uniquement.
- Fonctions publiques documentées. Mesurer avant d'optimiser (puffin ou `patine_cli bench`).

## Interdits
- Jamais les mots « Substance » ou « Adobe » dans le produit, le code ou la doc utilisateur.
- Jamais d'assets, d'icônes ou de formats de fichiers Adobe (.spp, .sbsar, .sbs, .sppr).
- Jamais d'asset sans licence claire : CC0 ou généré, avec métadonnées de licence.
- Ne jamais désactiver ou affaiblir un test pour le faire passer. Si un test est faux, explique pourquoi avant de le modifier.

## Machine de développement
- Windows avec RTX 3080 Ti (référence haute) et portable RTX A2000 (référence basse).
- Les budgets de performance sont dans `docs/PLAN.md`.