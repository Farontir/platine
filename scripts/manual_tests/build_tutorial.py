"""Builds the French manual-test tutorial (HTML, then PDF) from the captures.

Run from the repository root after capture_tutorial.py:
    python scripts/manual_tests/build_tutorial.py

Writes manual_tests/tutorial/ (HTML and cropped images) and
manual_tests/Patine-tutoriel-M0-M3.pdf, printed by Microsoft Edge (or Chrome) headless.
"""

import html
import os
import re
import shutil
import subprocess
import sys

from PIL import Image

CAPTURES = os.path.join("manual_tests", "captures")
OUT = os.path.join("manual_tests", "tutorial")
IMAGES = os.path.join(OUT, "img")
PDF = os.path.join("manual_tests", "Patine-tutoriel-M0-M3.pdf")

# Regions of the 1600 x 900 captures.
FULL = None
VIEWPORT = (14, 61, 1174, 628)
DIALOG = (520, 340, 1080, 560)
ABOUT = (640, 380, 960, 520)
RIGHT_TOP = (1178, 30, 1600, 260)
RIGHT_BOTTOM = (1178, 405, 1600, 790)
CONSOLE = (0, 640, 1180, 820)
STATUS = (0, 872, 700, 900)
HISTORY = (1178, 405, 1600, 575)
RECOVERY = (520, 380, 1080, 540)

BROWSERS = [
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    "msedge",
    "google-chrome",
    "chromium",
]


def crop(name, region, suffix):
    """Crops a capture into img/<name>-<suffix>.jpg and returns its relative path."""
    source = os.path.join(CAPTURES, f"{name}.png")
    image = Image.open(source).convert("RGB")
    if region is not None:
        image = image.crop(region)
    if suffix == "status":
        # The status bar text is small: enlarge it so it stays readable on paper.
        image = image.resize((image.width * 3, image.height * 3), Image.LANCZOS)
    file_name = f"{name}-{suffix}.jpg"
    image.save(os.path.join(IMAGES, file_name), quality=90)
    return f"img/{file_name}"


def figure(name, region, suffix, caption, width="100%"):
    path = crop(name, region, suffix)
    return (
        f'<figure style="width:{width}"><img src="{path}" alt="">'
        f"<figcaption>{caption}</figcaption></figure>"
    )


def grid(items, columns=2):
    """Viewport crops side by side: items are (capture name, caption)."""
    cells = "".join(figure(name, VIEWPORT, "view", caption) for name, caption in items)
    return f'<div class="grid c{columns}">{cells}</div>'


def code(text):
    return f"<pre>{html.escape(text)}</pre>"


def table(header, rows):
    head = "".join(f"<th>{h}</th>" for h in header)
    body = "".join("<tr>" + "".join(f"<td>{c}</td>" for c in row) + "</tr>" for row in rows)
    return f"<table><thead><tr>{head}</tr></thead><tbody>{body}</tbody></table>"


def test(steps, expected):
    """A 'how to test' box: numbered steps and the expected result."""
    items = "".join(f"<li>{s}</li>" for s in steps)
    return (
        f'<div class="test"><div class="label">Tester</div><ol>{items}</ol>'
        f'<div class="label">Résultat attendu</div><p>{expected}</p></div>'
    )


def checklist(items):
    return '<ul class="check">' + "".join(f"<li>{i}</li>" for i in items) + "</ul>"


ASSETS = [
    ["<b>FlightHelmet.gltf</b> + .bin", "glTF à buffer séparé ; 94 722 triangles, 6 objets, 6 matériaux",
     "Khronos glTF Sample Assets — Gary Hsu", "glTF avec fichier .bin externe, 6 texture sets, ombres, survol"],
    ["<b>BoomBox.glb</b>", "glTF binaire ; 6 036 triangles, 1 matériau",
     "Khronos glTF Sample Assets — Microsoft", "format .glb, un seul texture set"],
    ["<b>vintage_suitcase_1k.fbx</b>", "FBX ; 16 466 triangles, 8 objets, 2 matériaux",
     "Poly Haven — Maximilian Schuster", "FBX multi-objets, fil de fer, projection"],
    ["<b>Lantern_01_1k.fbx</b>", "FBX ; 33 902 triangles, 2 matériaux (laiton, verre)",
     "Poly Haven — Rajil Jose Macatangay", "matériau d'aperçu, environnements, exposition, fond"],
    ["<b>qwantani_noon_puresky_2k.exr</b>", "HDRI OpenEXR 2K, soleil de midi très intense",
     "Poly Haven — Greg Zaal, Jarod Guest", "chargement EXR, tone mapping des hautes lumières"],
    ["<b>brown_photostudio_02_2k.hdr</b>", "HDRI Radiance 2K, studio photo",
     "Poly Haven — Sergej Majboroda", "chargement HDR, éclairage de studio"],
]

GENERATED = [
    ["<b>generated/vintage_suitcase.obj</b> + .mtl", "conversion OBJ de la valise",
     "import OBJ, comparaison avec le FBX"],
    ["<b>generated/suzanne_1m.glb</b>", "tête de singe de Blender subdivisée : 1 007 616 triangles",
     "gros mesh : import non bloquant, rendu fluide"],
    ["<b>generated/unsupported.stl</b>", "fichier d'un format non pris en charge",
     "message d'erreur de format"],
]

CONTROLS = [
    ["Alt + clic gauche glissé", "orbite autour du mesh"],
    ["Alt + clic milieu glissé", "panoramique"],
    ["Alt + clic droit glissé, ou molette", "zoom"],
    ["Maj + clic droit glissé", "rotation de l'environnement"],
    ["F (ou Affichage › Cadrer le mesh)", "recadre le mesh entier"],
]

SHORTCUTS = [
    ["Ctrl+N", "Fichier › Nouveau projet…"],
    ["Ctrl+O", "Fichier › Ouvrir… (M3)"],
    ["Ctrl+S, Ctrl+Maj+S", "Fichier › Enregistrer, Enregistrer sous… (M3)"],
    ["Ctrl+Q", "Fichier › Quitter"],
    ["Ctrl+Z, Ctrl+Y", "Édition › Annuler, Rétablir (M3)"],
    ["F", "Affichage › Cadrer le mesh"],
    ["Ctrl+Maj+P", "Affichage › Profileur"],
    ["—", "Affichage › Orthographique, Fil de fer, Ombres, Réinitialiser la disposition"],
    ["—", "Langue › English / Français ; Aide › À propos de Patine"],
]

OPTIONS = [
    ["<code>[mesh]</code>", "ouvre ce mesh comme projet"],
    ["<code>--language fr|en</code>", "langue de l'interface"],
    ["<code>--window LxH</code>", "taille de la fenêtre"],
    ["<code>--tab PANNEAU</code>", "met un panneau au premier plan (texture-sets, display, console…)"],
    ["<code>--hdri CHEMIN|studio</code>", "environnement"],
    ["<code>--rotation DEG</code>, <code>--exposure EV</code>", "rotation de l'environnement, exposition"],
    ["<code>--tonemap agx|aces|neutral|linear</code>", "opérateur de tone mapping"],
    ["<code>--background env:FLOU|solid:R,V,B</code>", "fond"],
    ["<code>--material R,V,B,METALLIC,ROUGHNESS</code>", "matériau d'aperçu"],
    ["<code>--wireframe</code>, <code>--no-shadows</code>, <code>--ortho</code>, <code>--fov DEG</code>", "superpositions et projection"],
    ["<code>--orbit LACET,TANGAGE</code>, <code>--zoom FACTEUR</code>", "position de la caméra après cadrage"],
    ["<code>--hover X,Y</code>", "simule le curseur sur ce pixel de la vue 3D"],
    ["<code>--dialog new-project|about</code>, <code>--dialog-path</code>, <code>--dialog-create</code>", "ouvre et remplit un dialogue"],
    ["<code>--capture PNG</code>, <code>--capture-frames N</code>", "enregistre la fenêtre en PNG puis quitte"],
    ["<code>projet.patine</code>", "ouvre un projet au lieu d'importer un mesh"],
    ["<code>--edit rotation=DEG|exposure=EV|rename=SET:NOM|undo=N</code>", "applique des commandes au projet ouvert (une entrée d'historique chacune)"],
    ["<code>--autosave-interval SECS</code>", "délai entre deux autosaves (300 par défaut)"],
    ["variable <code>PATINE_AUTOSAVE_DIR</code>", "dossier des autosaves (par défaut %APPDATA%\\Patine\\autosave)"],
]


def body():
    parts = []
    add = parts.append

    # Cover.
    add('<section class="cover">'
        '<div class="kicker">Tutoriel et cahier de tests manuels</div>'
        "<h1>Patine</h1>"
        '<p class="subtitle">Texturing PBR par calques sur mesh 3D<br>'
        "Fonctionnalités des jalons M0 à M3</p>"
        + figure("helmet_texture_sets", FULL, "full", "Le casque d'aviateur importé, panneau Texture sets au premier plan.", "92%")
        + '<p class="meta">Version 0.1.0 — 11 octobre 2026 — captures réalisées sur RTX 3080 Ti (Vulkan), '
        "interface en français, fenêtre 1600 × 900.</p></section>")

    # Contents.
    add('<section><h2>Sommaire</h2><ol class="toc">'
        "<li>Préparer les fichiers de test</li>"
        "<li>M0 — L'interface : panneaux, langue, console, profileur</li>"
        "<li>M1 — Import de meshes : nouveau projet, formats, texture sets, erreurs</li>"
        "<li>M2 — Viewport PBR : caméra, environnements, tone mapping, ombres, fil de fer, survol</li>"
        "<li>M3 — Document : annuler, historique, enregistrer, autosave</li>"
        "<li>Checklist complète des tests manuels</li>"
        "<li>Annexe : options de lancement et régénération de ce document</li>"
        "</ol>"
        "<p>Chaque fonctionnalité est présentée en trois temps : ce qu'elle fait, comment la tester, "
        "et le résultat attendu, illustré par une capture de l'application. Les captures ont été produites "
        "par le mode capture de <code>patine_app</code> : elles sont reproductibles à l'identique "
        "(annexe).</p>"
        "<h3>Lancer l'application</h3>"
        "<p>Le build <i>release</i> est conseillé pour les gros meshes et les HDRI 2K ; le premier argument "
        "ouvre directement un mesh comme projet.</p>"
        + code("cargo build --release -p patine_app -p patine_cli\n"
               "cargo run --release -p patine_app\n"
               "cargo run --release -p patine_app -- manual_tests/assets/FlightHelmet.gltf")
        + "</section>")

    # 1. Files.
    add('<section><h2>1. Préparer les fichiers de test</h2>'
        "<p>Les fichiers téléchargés sont placés dans <code>manual_tests/assets/</code>, un dossier "
        "ignoré par git. Tous sont sous licence <b>CC0</b> (domaine public) ; leurs sources et leurs "
        "empreintes SHA-256 sont dans <code>manual_tests/assets/LICENSE.md</code>.</p>"
        "<h3>Fichiers téléchargés</h3>"
        + table(["Fichier", "Contenu", "Source — auteur (CC0)", "Sert à tester"], ASSETS)
        + "<h3>Fichiers générés localement</h3>"
        "<p>Ils sont produits par Blender à partir des fichiers ci-dessus, sans autre téléchargement :</p>"
        + code("blender -b --factory-startup --python scripts/manual_tests/generate_assets.py")
        + table(["Fichier (dans manual_tests/assets)", "Contenu", "Sert à tester"], GENERATED)
        + "<h3>Fichiers déjà dans le dépôt</h3><ul>"
        "<li><code>tests/fixtures/meshes/</code> : les fixtures du M1 (<i>cube_materials</i>, <i>multi_object</i>, "
        "<i>concave_ngon</i>, <i>no_uv</i>, <i>vertex_colors</i>), chacune en .glb, .gltf, .fbx et .obj.</li>"
        "<li><code>assets/environments/</code> : deux HDRI CC0 livrées avec l'application, "
        "<i>studio_small_08</i> et <i>kloofendal_48d_partly_cloudy_puresky</i>, en 1K.</li></ul>"
        "</section>")

    # 2. M0.
    add("<section><h2>2. M0 — L'interface</h2>"
        "<h3>2.1 Fenêtre et panneaux</h3>"
        "<p>La fenêtre est découpée en neuf panneaux ancrables : <b>Vue 3D</b> et <b>Vue 2D</b> au centre, "
        "<b>Calques</b> et <b>Texture sets</b> en haut à droite, <b>Propriétés</b>, <b>Paramètres d'affichage</b> "
        "et <b>Historique</b> en bas à droite, <b>Bibliothèque</b> et <b>Console</b> en bas. Les panneaux des "
        "jalons suivants affichent « Rien ici pour l'instant. ». La barre d'état indique la carte graphique "
        "et l'API utilisées, puis le projet ouvert.</p>"
        + figure("interface_fr", FULL, "full", "L'interface au démarrage, sans projet : la vue 3D montre l'environnement seul.")
        + test(["Faire glisser l'onglet d'un panneau vers un autre bord : il s'ancre à l'endroit indiqué.",
                "Le faire glisser hors de la fenêtre : il devient une fenêtre flottante, que l'on peut réancrer.",
                "Affichage › Réinitialiser la disposition.",
                "Redimensionner, minimiser puis restaurer la fenêtre."],
               "Chaque panneau se déplace, se détache et se réancre ; la réinitialisation restaure la disposition "
               "ci-dessus ; aucune image figée ni plantage au redimensionnement.")
        + "<h3>2.2 Langue</h3>"
        "<p>La langue est détectée depuis le système au premier lancement. Le menu <b>Langue</b> bascule toute "
        "l'interface entre le français et l'anglais, sans redémarrer.</p>"
        + figure("interface_en", FULL, "full", "La même interface en anglais (Langue › English).", "86%")
        + test(["Langue › English, puis Langue › Français.",
                "Ouvrir un dialogue et un panneau rempli dans chaque langue."],
               "Menus, panneaux, dialogues, messages d'erreur et barre d'état changent de langue immédiatement.")
        + "</section>")

    add("<section>"
        "<h3>2.3 À propos</h3>"
        "<p>Aide › À propos de Patine affiche le nom, la description et la version.</p>"
        + figure("about", ABOUT, "dialog", "Le dialogue À propos.", "45%")
        + "<h3>2.4 Console</h3>"
        "<p>La console reçoit les journaux de l'application : démarrage, carte graphique choisie, capacités "
        "détectées, imports. La liste <b>Niveau minimal</b> filtre les messages (Erreurs, Avertissements, "
        "Informations, Débogage, Trace) et <b>Effacer</b> la vide.</p>"
        + figure("helmet_console", CONSOLE, "console", "La console après l'import du casque : version, projet créé, durées d'import, carte graphique et capacités.")
        + test(["Ouvrir l'onglet Console au démarrage.", "Changer le niveau minimal, puis cliquer Effacer."],
               "Les lignes « GPU adapter selected » et « GPU ready » sont présentes ; le filtre masque les niveaux "
               "inférieurs ; Effacer vide la liste.")
        + "<h3>2.5 Profileur</h3>"
        "<p>Affichage › Profileur (Ctrl+Maj+P) démarre le serveur de profilage ; la console indique le port. "
        "On peut aussi le démarrer dès le lancement :</p>"
        + code("PATINE_PROFILE=1 cargo run -p patine_app\npuffin_viewer")
        + test(["Activer le profileur, puis lancer puffin_viewer et se connecter au port indiqué."],
               "Les frames arrivent en continu dans puffin_viewer, avec le détail des étapes.")
        + "<h3>2.6 Raccourcis</h3>"
        + table(["Raccourci", "Action"], SHORTCUTS)
        + "<p>Les raccourcis sans modificateur (comme F) sont ignorés pendant la saisie dans un champ de texte.</p>"
        "</section>")

    # 3. M1.
    add("<section><h2>3. M1 — Import de meshes</h2>"
        "<h3>3.1 Nouveau projet</h3>"
        "<p>Fichier › Nouveau projet… (Ctrl+N) ouvre le dialogue de création. <b>Parcourir…</b> ouvre le "
        "sélecteur natif filtré sur .glb, .gltf, .fbx et .obj. On choisit la <b>résolution</b> des texture sets "
        "et la <b>convention de normal map</b> (OpenGL Y+ ou DirectX Y−), puis <b>Créer</b>. L'import tourne en "
        "arrière-plan : l'interface reste utilisable.</p>"
        + figure("new_project_dialog", DIALOG, "dialog", "Le dialogue Nouveau projet, avec le casque sélectionné.", "62%")
        + "<h3>3.2 Texture sets</h3>"
        "<p>Chaque matériau du mesh devient un <b>texture set</b>. Le panneau Texture sets affiche le nom du mesh, "
        "ses nombres de triangles et de sommets, puis une ligne par texture set avec sa résolution et son nombre "
        "de triangles. Le mesh apparaît dans la vue 3D, cadré.</p>"
        + '<div class="grid c2a">'
        + figure("helmet_texture_sets", VIEWPORT, "view", "Le casque dans la vue 3D.")
        + figure("helmet_texture_sets", RIGHT_TOP, "panel", "Ses 6 texture sets.")
        + "</div>"
        + test(["Ctrl+N, Parcourir…, choisir <code>manual_tests/assets/FlightHelmet.gltf</code>, Créer.",
                "Ouvrir l'onglet Texture sets."],
               "6 texture sets en 2048 × 2048 : LensesMat, LeatherPartsMat, MetalPartsMat, GlassPlasticMat, "
               "RubberWoodMat, HoseMat ; 94 722 triangles et 55 406 sommets au total.")
        + "</section>")

    add("<section>"
        "<h3>3.3 Formats pris en charge</h3>"
        "<p>glTF (fichier .gltf avec .bin, ou .glb binaire), FBX et OBJ. Le même objet importé en FBX et en OBJ "
        "doit donner le même résultat.</p>"
        + grid([("boombox_glb", "BoomBox.glb : glTF binaire, 1 texture set."),
                ("suitcase_fbx", "vintage_suitcase_1k.fbx : FBX, 8 objets, 2 texture sets."),
                ("suitcase_obj", "generated/vintage_suitcase.obj : la même valise en OBJ."),
                ("fixture_multi_object", "Fixture multi_object.glb : plusieurs objets, plusieurs matériaux.")])
        + test(["Importer successivement BoomBox.glb, vintage_suitcase_1k.fbx et generated/vintage_suitcase.obj.",
                "Importer chaque fixture de <code>tests/fixtures/meshes/</code> dans ses quatre formats."],
               "Chaque mesh apparaît cadré, éclairé, sans facettes inattendues ; la valise FBX et la valise OBJ "
               "ont les mêmes texture sets ; une fixture donne les mêmes comptes dans ses quatre formats.")
        + "<h3>3.4 Gros mesh</h3>"
        "<p>L'import d'un mesh de plus d'un million de triangles ne fige pas l'interface. La console détaille la "
        "durée de chaque étape : lecture, nettoyage, normales, tangentes, soudure.</p>"
        + figure("suzanne_1m_console", CONSOLE, "console", "Import de generated/suzanne_1m.glb : 1 007 616 triangles.")
        + test(["Importer <code>generated/suzanne_1m.glb</code> et déplacer une fenêtre de panneau pendant l'import."],
               "L'interface répond pendant l'import ; la console affiche triangles=1007616.")
        + "</section>")

    add("<section>"
        "<h3>3.5 Messages d'erreur</h3>"
        "<p>Un mesh invalide n'est jamais importé à moitié : le dialogue reste ouvert et explique le problème "
        "dans la langue de l'interface.</p>"
        + '<div class="grid c2">'
        + figure("error_missing_uvs", DIALOG, "dialog", "Fixture no_uv.fbx : l'objet sans UV est nommé, avec la marche à suivre.")
        + figure("error_unsupported", DIALOG, "dialog", "generated/unsupported.stl : format refusé, formats acceptés listés.")
        + "</div>"
        + test(["Importer <code>tests/fixtures/meshes/no_uv.fbx</code>, puis <code>generated/unsupported.stl</code>.",
                "Recommencer en anglais."],
               "Les deux messages ci-dessus s'affichent en rouge dans le dialogue, sans plantage, dans les deux langues.")
        + "<h3>3.6 Inspection en ligne de commande</h3>"
        "<p><code>patine_cli inspect</code> importe un mesh sans interface et affiche ses statistiques : utile pour "
        "comparer avec la console de l'application.</p>"
        + code("cargo run --release -p patine_cli -- inspect manual_tests/assets/Lantern_01_1k.fbx")
        + table(["Fichier", "Triangles", "Objets", "Matériaux"],
                [["FlightHelmet.gltf", "94 722", "6", "6"],
                 ["BoomBox.glb", "6 036", "1", "1"],
                 ["vintage_suitcase_1k.fbx", "16 466", "8", "2"],
                 ["Lantern_01_1k.fbx", "33 902", "—", "2"],
                 ["generated/suzanne_1m.glb", "1 007 616", "1", "1"]])
        + "</section>")

    # 4. M2.
    add("<section><h2>4. M2 — Viewport PBR</h2>"
        "<p>La vue 3D rend le mesh avec un modèle physique (métal/rugosité, GGX), éclairé par une image "
        "d'environnement (HDRI), avec antialiasing MSAA 4x, ombres et tone mapping. Au M2 le mesh porte un "
        "matériau constant, l'aperçu des textures arrive au M4.</p>"
        "<h3>4.1 Caméra</h3>"
        + table(["Geste", "Effet"], CONTROLS)
        + test(["Ouvrir un mesh, puis essayer chaque geste.", "Après s'être éloigné, appuyer sur F."],
               "Mouvements fluides, sans saut ; le zoom ne traverse pas le mesh ; F recadre le mesh entier. "
               "Sous Windows, vérifier qu'Alt seul n'ouvre pas le menu système de la fenêtre.")
        + "<h3>4.2 Panneau Paramètres d'affichage</h3>"
        "<p>Il regroupe tous les réglages du rendu : environnement (liste, Charger une HDRI…, rotation, exposition, "
        "tone mapping), fond, superpositions (ombres, fil de fer), projection et champ de vision, et le matériau "
        "d'aperçu (couleur de base, metallic, roughness). Le rendu suit chaque réglage en direct.</p>"
        + '<div class="grid c2a">'
        + figure("display_panel", VIEWPORT, "view", "La lanterne avec les réglages par défaut : studio généré, AgX, 0 EV.")
        + figure("display_panel", RIGHT_BOTTOM, "panel", "Le panneau Paramètres d'affichage.")
        + "</div></section>")

    add("<section>"
        "<h3>4.3 Matériau d'aperçu</h3>"
        "<p><b>Metallic</b> à 1 donne une surface métallique qui reflète l'environnement teinté par la couleur de base ; "
        "à 0, une surface diélectrique (plastique, bois). <b>Roughness</b> règle la netteté des reflets.</p>"
        + grid([("material_metal", "Laiton : couleur (0,95 ; 0,72 ; 0,35), metallic 1, roughness 0,25."),
                ("material_plastic", "Plastique rouge : metallic 0, roughness 0,45."),
                ("material_rough_metal", "Métal brossé : metallic 1, roughness 0,85 : reflets diffus."),
                ("display_panel", "Matériau par défaut : gris clair, metallic 0, roughness 0,5.")])
        + test(["Ouvrir Lantern_01_1k.fbx, onglet Paramètres d'affichage.",
                "Faire varier metallic de 0 à 1, puis roughness de 0 à 1, et changer la couleur de base."],
               "Le reflet de l'environnement devient net à roughness 0 et diffus à 1 ; à metallic 1 la couleur teinte "
               "les reflets ; aucune zone noire ou saturée anormale.")
        + "</section>")

    add("<section>"
        "<h3>4.4 Environnements</h3>"
        "<p>La liste propose le studio généré par l'application et les deux HDRI livrées ; <b>Charger une HDRI…</b> "
        "accepte les fichiers .hdr et .exr. L'environnement éclaire le mesh (diffus et reflets) et peut servir de fond.</p>"
        + grid([("env_studio_generated", "Studio (généré) : éclairage doux, par défaut."),
                ("env_studio_small", "studio_small_08 (livrée)."),
                ("env_kloofendal", "kloofendal_48d_partly_cloudy (livrée) : ciel nuageux."),
                ("env_photostudio", "brown_photostudio_02.hdr (chargée)."),
                ("env_noon_exr", "qwantani_noon_puresky.exr (chargée) : soleil de midi.")])
        + test(["Choisir chaque environnement de la liste.",
                "Charger une HDRI… : <code>brown_photostudio_02_2k.hdr</code>, puis <code>qwantani_noon_puresky_2k.exr</code>."],
               "« Chargement… » s'affiche brièvement, puis l'éclairage et le fond changent ; le soleil de l'EXR ne "
               "crée ni pixels noirs ni couleurs aberrantes.")
        + "</section>")

    add("<section>"
        "<h3>4.5 Rotation de l'environnement</h3>"
        "<p>Le curseur Rotation (ou Maj + clic droit glissé dans la vue) fait tourner l'environnement autour de "
        "l'axe vertical : la direction de la lumière et le fond tournent ensemble.</p>"
        + grid([("rotation_0", "Kloofendal, rotation 0°."),
                ("rotation_180", "Kloofendal, rotation 180° : la lumière vient de l'autre côté.")])
        + "<h3>4.6 Exposition</h3>"
        "<p>L'exposition, en EV, multiplie la lumière avant le tone mapping : +1 EV double la luminosité.</p>"
        + grid([("exposure_minus2", "−2 EV."), ("exposure_plus2", "+2 EV.")])
        + test(["Maj + clic droit glissé dans la vue, puis curseur Rotation.",
                "Exposition à −2, 0, puis +2 EV."],
               "La lumière et le reflet tournent avec le fond, sans saccade ; l'exposition assombrit ou éclaircit "
               "l'image entière, fond compris.")
        + "</section>")

    add("<section>"
        "<h3>4.7 Tone mapping</h3>"
        "<p>L'opérateur convertit la lumière (non bornée) en couleurs affichables. Les différences se voient sur les "
        "hautes lumières : ici la tête de singe orange, éclairée par le soleil de midi à +1,5 EV.</p>"
        + grid([("tonemap_agx", "AgX (par défaut) : hautes lumières qui tendent vers le blanc, teintes préservées."),
                ("tonemap_aces", "ACES : plus contrasté et saturé."),
                ("tonemap_neutral", "PBR Neutral : couleurs de base fidèles."),
                ("tonemap_linear", "Linéaire : simple écrêtage, les hautes lumières saturent en aplats.")])
        + test(["Ouvrir generated/suzanne_1m.glb avec l'HDRI EXR, exposition +1,5 EV.",
                "Passer par les quatre opérateurs."],
               "Les zones éclairées par le soleil changent nettement d'un opérateur à l'autre ; Linéaire montre des "
               "aplats écrêtés que les trois autres évitent.")
        + "</section>")

    add("<section>"
        "<h3>4.8 Fond</h3>"
        "<p>Le fond est soit l'environnement, plus ou moins flouté (curseur Flou), soit une couleur unie. "
        "Il ne change pas l'éclairage du mesh.</p>"
        + grid([("background_blur", "Environnement flouté (flou 0,7)."),
                ("background_solid", "Couleur unie gris foncé.")])
        + "<h3>4.9 Ombres</h3>"
        "<p>La lumière principale de l'HDRI (le soleil, ou la source la plus forte) est extraite automatiquement "
        "et projette des ombres. Désactivées, elles laissent l'éclairage inchangé hors des zones occultées.</p>"
        + grid([("shadows_on", "Ombres activées : la base du casque reçoit l'ombre du tuyau et de la tête."),
                ("shadows_off", "Ombres désactivées (Affichage › Ombres).")])
        + test(["Ouvrir le casque avec l'HDRI kloofendal et une rotation de 180°.",
                "Basculer Affichage › Ombres."],
               "Avec ombres, une ombre portée cohérente avec la direction du soleil apparaît sur le socle ; "
               "sans ombres, seule cette ombre disparaît : aucune face ne devient noire ou bleutée.")
        + '<p class="note">Défaut trouvé pendant la préparation de ce tutoriel : avec un soleil intense (HDRI EXR), '
        "désactiver les ombres noircissait les faces opposées au soleil. Corrigé et couvert par un test automatique.</p>"
        + "</section>")

    add("<section>"
        "<h3>4.10 Fil de fer</h3>"
        "<p>Affichage › Fil de fer superpose les arêtes des polygones d'origine. Les diagonales de la triangulation "
        "restent cachées : un quad apparaît comme un quad, un n-gone comme un n-gone.</p>"
        + grid([("wireframe", "La valise en fil de fer."),
                ("fixture_ngon_wireframe", "Fixture concave_ngon.obj vue de dessus : un n-gone concave en L et un quad ; seuls le contour et leur arête commune sont tracés.")])
        + "<h3>4.11 Projection</h3>"
        "<p>Perspective, avec un champ de vision réglable, ou orthographique (Affichage › Orthographique), sans "
        "déformation des parallèles.</p>"
        + grid([("projection_perspective", "Perspective, champ de vision 70°."),
                ("projection_ortho", "Orthographique, même point de vue.")])
        + test(["Activer le fil de fer sur la valise et sur la fixture concave_ngon.",
                "Basculer la projection et faire varier le champ de vision."],
               "Pas de diagonale dans les quads ni dans le n-gone ; en orthographique les arêtes parallèles le restent, "
               "et le zoom fonctionne toujours.")
        + "</section>")

    add("<section>"
        "<h3>4.12 Survol d'un triangle</h3>"
        "<p>Sous le curseur, la barre d'état indique le triangle, le polygone d'origine et l'objet touchés. "
        "Ce sera la base de la peinture et de la sélection.</p>"
        + figure("picking_hover", VIEWPORT, "view", "Le curseur sur le socle du casque.", "80%")
        + figure("picking_hover", STATUS, "status", "La barre d'état correspondante : triangle 53987, polygone 53987, objet RubberWood_low.")
        + test(["Ouvrir le casque, survoler différentes pièces, puis le fond."],
               "Le numéro de triangle change en continu, l'objet correspond à la pièce survolée ; sur le fond, "
               "l'indication disparaît.")
        + "<h3>4.13 Mesure des performances</h3>"
        + code("cargo run --release -p patine_cli -- bench viewport")
        + "<p>Rend hors écran une sphère de 500 000 triangles en 1920 × 1080, MSAA 4x, ombres et IBL actives. "
        "Budget : au moins 120 images/s sur la RTX 3080 Ti (mesuré : 3 372) et au moins 60 sur la RTX A2000 "
        "(<b>à mesurer</b>).</p></section>")

    # 5. M3.
    add("<section><h2>5. M3 — Document, annulation et sauvegarde</h2>"
        "<p>Depuis le M3, toute modification du projet passe par une <b>commande</b> annulable, et le projet "
        "s'enregistre dans un fichier <code>.patine</code>. Les réglages d'environnement (HDRI, rotation, "
        "exposition) font désormais partie du projet ; le tone mapping, le fond, les ombres, le fil de fer et "
        "la caméra restent des préférences d'affichage.</p>"
        "<h3>5.1 Annuler, rétablir et panneau Historique</h3>"
        "<p>Édition › Annuler (Ctrl+Z) et Rétablir (Ctrl+Y). Le panneau <b>Historique</b> liste les commandes, "
        "de la plus ancienne à la plus récente, sous la ligne « Ouverture du projet ». L'état courant est en "
        "surbrillance, les commandes annulées sont grisées ; un clic sur une ligne revient à cet état. Un geste "
        "continu (glisser un curseur, Maj + clic droit) ne crée qu'une seule entrée.</p>"
        + '<div class="grid c2a">'
        + figure("m3_history", VIEWPORT, "view", "Rotation 60°, exposition 0,7, renommage, rotation 140° puis Ctrl+Z.")
        + figure("m3_history", HISTORY, "panel", "L'Historique : la dernière rotation, annulée, est grisée.")
        + "</div>"
        + test(["Ouvrir le casque. Faire glisser le curseur Rotation, puis Exposition, puis Maj + clic droit dans la vue.",
                "Ctrl+Z deux fois, Ctrl+Y une fois, puis cliquer « Ouverture du projet » et la dernière ligne.",
                "Après un Ctrl+Z, modifier l'exposition."],
               "Une entrée par geste ; le rendu suit chaque annulation, HDRI comprise ; une nouvelle modification "
               "après une annulation efface les entrées grisées.")
        + "<h3>5.2 Renommer un texture set</h3>"
        "<p>Double-clic sur un nom dans le panneau Texture sets, puis Entrée (ou clic ailleurs) pour valider, "
        "Échap pour annuler. Pendant la saisie, Ctrl+Z annule la frappe, pas le document.</p>"
        + '<div class="grid c2a">'
        + figure("m3_renamed", VIEWPORT, "view", "Le casque après le renommage.")
        + figure("m3_renamed", RIGHT_TOP, "panel", "LensesMat renommé en « Verres ».")
        + "</div>"
        + test(["Double-cliquer LensesMat, taper « Verres », Entrée.", "Ctrl+Z, puis Ctrl+Y."],
               "Le nom change, revient, puis change à nouveau ; l'Historique affiche « Renommer le texture set en « Verres » ».")
        + "</section>")

    add("<section>"
        "<h3>5.3 Enregistrer et ouvrir</h3>"
        "<p>Fichier › Enregistrer (Ctrl+S), Enregistrer sous… (Ctrl+Maj+S) et Ouvrir… (Ctrl+O). L'enregistrement "
        "se fait en arrière-plan sur une copie du projet : on peut continuer à travailler, la barre d'état "
        "affiche « Enregistrement… ». Le titre de la fenêtre porte le nom du projet, suivi de <b>*</b> tant "
        "qu'il y a des modifications non enregistrées. Une HDRI chargée par l'utilisateur est copiée dans le "
        "fichier : le projet reste complet si on le déplace.</p>"
        + figure("m3_opened", FULL, "full", "manual_tests/casque.patine rouvert : même mesh, mêmes texture sets ; l'Historique repart de « Ouverture du projet ».")
        + test(["Ouvrir le casque, charger brown_photostudio_02_2k.hdr, tourner l'environnement, Ctrl+S.",
                "Fermer Patine, déplacer le .patine dans un autre dossier, l'ouvrir (Ctrl+O, ou en argument de patine_app).",
                "Ouvrir un fichier qui n'est pas un projet (un .glb renommé en .patine)."],
               "Le projet se rouvre à l'identique, HDRI et rotation compris, et le titre perd son * après "
               "l'enregistrement ; le faux projet donne « ce fichier n'est pas un projet Patine ».")
        + "<h3>5.4 Modifications non enregistrées</h3>"
        "<p>Nouveau projet, Ouvrir, Quitter et la fermeture de la fenêtre demandent quoi faire des modifications "
        "non enregistrées : <b>Enregistrer</b> (puis continuer), <b>Ne pas enregistrer</b> ou <b>Annuler</b>.</p>"
        + test(["Modifier l'exposition, puis Ctrl+Q : essayer chacun des trois boutons.",
                "Recommencer avec la croix de la fenêtre, Ctrl+N et Ctrl+O."],
               "Annuler ne fait rien ; Ne pas enregistrer continue ; Enregistrer demande un fichier si le projet "
               "n'en a pas, enregistre, puis continue.")
        + "</section>")

    add("<section>"
        "<h3>5.5 Autosave et récupération après un crash</h3>"
        "<p>Toutes les 5 minutes, si le projet a des modifications non enregistrées, Patine en écrit une copie "
        "dans <code>%APPDATA%\\Patine\\autosave</code>. Une fermeture normale la supprime. Après un crash, "
        "le lancement suivant propose de <b>récupérer</b> le projet (marqué modifié, avec son chemin d'origine) "
        "ou de <b>supprimer la copie</b>. Deux instances ouvertes en même temps ne se gênent pas.</p>"
        + figure("m3_recovery", RECOVERY, "dialog", "Le dialogue de récupération au lancement suivant un crash.", "62%")
        + test(["Lancer <code>patine_app manual_tests/assets/FlightHelmet.gltf --autosave-interval 10</code>.",
                "Modifier l'exposition, attendre 15 s, puis tuer patine_app.exe dans le Gestionnaire des tâches.",
                "Relancer Patine, cliquer Récupérer.",
                "Ouvrir deux instances en même temps, en fermer une normalement, relancer."],
               "Le projet revient avec l'exposition modifiée et un * dans le titre ; aucune récupération n'est "
               "proposée entre deux instances vivantes, ni après une fermeture normale.")
        + "<h3>5.6 Mesure des fichiers projet</h3>"
        + code("cargo run --release -p patine_cli -- bench project manual_tests/assets/generated/suzanne_1m.glb")
        + "<p>Crée un projet, l'enregistre, le rouvre, vérifie qu'il est identique et affiche les durées. "
        "Mesures sur la RTX 3080 Ti :</p>"
        + table(["Mesh", "Triangles", "Import", "Enregistrement", "Ouverture", "Fichier"],
                [["FlightHelmet.gltf", "94 722", "170 ms", "26 ms", "22 ms", "2,6 Mo"],
                 ["suzanne_1m.glb", "1 007 616", "1 876 ms", "229 ms", "105 ms", "25,9 Mo"]])
        + "<p><code>--out fichier.patine</code> garde le projet créé : c'est ainsi qu'est produit "
        "<code>manual_tests/casque.patine</code>.</p></section>")

    # 6. Checklist.
    add("<section><h2>6. Checklist complète des tests manuels</h2>"
        "<h3>M0 — Fondations</h3>"
        + checklist(["<code>cargo run -p patine_app</code> ouvre une fenêtre sans erreur dans le terminal.",
                     "Les 9 panneaux sont présents ; on peut les détacher, les déplacer et les réancrer.",
                     "Affichage › Réinitialiser la disposition restaure la disposition d'origine.",
                     "La console affiche les logs de démarrage (adaptateur GPU, backend).",
                     "Menu Langue : bascule FR ↔ EN de toute l'interface sans redémarrer.",
                     "Activer le profileur, lancer puffin_viewer et voir les frames arriver.",
                     "Redimensionner, minimiser puis restaurer la fenêtre : pas de crash, pas d'image figée.",
                     "Ctrl+Q quitte l'application.",
                     "Sur le portable RTX A2000 : mêmes vérifications."])
        + "<h3>M1 — Import de meshes</h3>"
        + checklist(["Ctrl+N ouvre le dialogue ; Parcourir ouvre le sélecteur natif filtré sur .glb, .gltf, .obj, .fbx.",
                     "Chaque fixture de <code>tests/fixtures/</code> se charge ; un texture set par matériau.",
                     "FlightHelmet.gltf : 6 texture sets ; BoomBox.glb : 1 ; valise FBX et OBJ : 2 chacune.",
                     "La fixture sans UV et le fichier .stl affichent un message clair, en français comme en anglais.",
                     "generated/suzanne_1m.glb (1 M triangles) se charge sans figer l'interface.",
                     "<code>patine_cli inspect</code> donne les mêmes comptes que la console."])
        + "<h3>M2 — Viewport PBR</h3>"
        + checklist(["Chaque mesh apparaît cadré, éclairé, sans facettes inattendues.",
                     "Alt + clic gauche, milieu, droit et molette : orbite, panoramique, zoom fluides ; F recadre.",
                     "Projection orthographique et champ de vision.",
                     "Environnements livrés, HDRI .hdr et .exr chargées, rotation (Maj + clic droit), exposition, "
                     "opérateurs : le rendu suit sans saccade.",
                     "Matériau d'aperçu : metallic, roughness et couleur de base réagissent comme décrit en 4.3.",
                     "Fond flouté et couleur unie.",
                     "Ombres : ombre portée cohérente avec le soleil ; sans ombres, aucune face noire ou bleutée.",
                     "Fil de fer : les quads et les n-gones apparaissent sans diagonale.",
                     "Survol : la barre d'état affiche le triangle sous le curseur.",
                     "Redimensionner et détacher le panneau Vue 3D.",
                     "<code>patine_cli bench viewport</code> sur la RTX A2000 : au moins 60 images/s."])
        + "<h3>M3 — Document, commandes et sauvegarde</h3>"
        + checklist(["Rotation, exposition, HDRI et renommage : une seule entrée d'Historique par geste.",
                     "Ctrl+Z et Ctrl+Y annulent et rétablissent, rendu compris ; un clic dans l'Historique revient à cet état.",
                     "Ctrl+Z pendant le renommage d'un texture set annule la saisie, pas le document.",
                     "Enregistrer, fermer, rouvrir : mesh, texture sets et environnement identiques ; le * du titre disparaît.",
                     "Une HDRI personnelle est restituée après le déplacement du .patine.",
                     "Quitter, fermer la fenêtre, ouvrir ou créer un projet avec des modifications : la confirmation s'affiche, "
                     "et chaque bouton fait ce qu'il dit.",
                     "Crash simulé avec <code>--autosave-interval 10</code> : la récupération est proposée et rend le travail.",
                     "Deux instances ouvertes en même temps : aucune ne propose de récupérer l'autre.",
                     "<code>patine_app projet.patine</code> ouvre le projet ; un fichier qui n'en est pas un donne un message clair."])
        + "</section>")

    # Appendix.
    add("<section><h2>7. Annexe</h2>"
        "<h3>Options de lancement de patine_app</h3>"
        "<p>Ces options servent aux captures et aux tests ; <code>patine_app --help</code> les liste.</p>"
        + table(["Option", "Effet"], OPTIONS)
        + "<p>Exemple : la capture du survol de ce document.</p>"
        + code("target/release/patine_app manual_tests/assets/FlightHelmet.gltf --orbit 30,10 \\\n"
               "    --hover 580,240 --language fr --window 1600x900 --capture survol.png")
        + "<h3>Régénérer ce document</h3>"
        + code("cargo build --release -p patine_app -p patine_cli\n"
               "python scripts/manual_tests/capture_tutorial.py\n"
               "python scripts/manual_tests/build_tutorial.py")
        + "<p>Le premier script produit les 43 captures dans <code>manual_tests/captures/</code> (et le projet "
        "<code>manual_tests/casque.patine</code>) ; le second les "
        "recadre et imprime ce PDF avec Microsoft Edge (ou Chrome) sans interface.</p>"
        "<h3>Crédits</h3>"
        "<p>Modèles et HDRI sous licence CC0 : Khronos Group glTF Sample Assets (Gary Hsu, Microsoft), "
        "Poly Haven (Maximilian Schuster, Rajil Jose Macatangay, Greg Zaal, Jarod Guest, Sergej Majboroda). "
        "Tête de singe « Suzanne » : primitive de Blender.</p></section>")

    return keep_headings("\n".join(parts))


# A subsection title, its paragraphs and its first illustration (figure, grid, code or
# table) are kept on the same page. Grids only contain figures, so the lazy match of
# their closing tag is exact.
KEEP = re.compile(
    r"<h3>.*?</h3>(?:<p>.*?</p>)*"
    r"(?:<div class=\"grid[^\"]*\">.*?</div>|<figure.*?</figure>|<pre>.*?</pre>|<table>.*?</table>)?",
    re.S,
)


def keep_headings(document):
    return KEEP.sub(lambda m: f'<div class="keep">{m.group(0)}</div>', document)


STYLE = """
@page { size: A4; margin: 14mm 14mm 16mm 14mm; }
* { box-sizing: border-box; }
body { font-family: "Segoe UI", "Helvetica Neue", Arial, sans-serif; font-size: 10pt; line-height: 1.45;
       color: #1d1f24; margin: 0; }
h1 { font-size: 44pt; margin: 0 0 4mm; letter-spacing: -0.5pt; color: #8a4b22; }
h2 { font-size: 18pt; color: #8a4b22; border-bottom: 1.5pt solid #d8b89c; padding-bottom: 2mm;
     margin: 0 0 4mm; break-before: page; }
.keep { break-inside: avoid; }
h3 { font-size: 12.5pt; margin: 5mm 0 2mm; color: #2c3440; break-after: avoid; }
p { margin: 0 0 2.5mm; }
code { font-family: Consolas, "Cascadia Mono", monospace; font-size: 8.8pt; background: #f2efea;
       padding: 0 1mm; border-radius: 1mm; }
pre { font-family: Consolas, "Cascadia Mono", monospace; font-size: 8.6pt; background: #1f242c; color: #e7e3da;
      padding: 2.5mm 3.5mm; border-radius: 1.5mm; white-space: pre-wrap; break-inside: avoid; margin: 0 0 3mm; }
figure { margin: 0 auto 3.5mm; break-inside: avoid; }
figure img { width: 100%; display: block; border: 0.5pt solid #b9bec7; border-radius: 1mm; }
figcaption { font-size: 8.6pt; color: #555c66; margin-top: 1.2mm; }
.grid { display: grid; gap: 3mm; margin-bottom: 2mm; }
.grid.c2 { grid-template-columns: 1fr 1fr; }
.grid.c2a { grid-template-columns: 1.75fr 1fr; align-items: start; }
.grid figure { width: 100% !important; margin: 0; }
table { border-collapse: collapse; width: 100%; font-size: 8.8pt; margin: 0 0 3mm; break-inside: auto; }
th { background: #2c3440; color: #fff; text-align: left; padding: 1.4mm 2mm; font-weight: 600; }
td { border-bottom: 0.5pt solid #d6d9de; padding: 1.1mm 2mm; vertical-align: top; }
tr { break-inside: avoid; }
.test { border-left: 2.5pt solid #8a4b22; background: #faf6f1; padding: 2.5mm 4mm; margin: 2mm 0 4mm;
        break-inside: avoid; }
.test .label { font-weight: 700; font-size: 8.5pt; text-transform: uppercase; letter-spacing: 0.6pt;
               color: #8a4b22; margin-bottom: 1mm; }
.test ol { margin: 0 0 2mm; padding-left: 5mm; }
.test p { margin: 0; }
.note { border: 0.8pt solid #d8b89c; background: #fff8ef; padding: 2mm 3.5mm; border-radius: 1mm; font-size: 9pt; }
ul.check { list-style: none; padding-left: 0; }
ul.check li { padding-left: 7mm; position: relative; margin-bottom: 1.6mm; break-inside: avoid; }
ul.check li::before { content: ""; position: absolute; left: 0; top: 0.8mm; width: 3.2mm; height: 3.2mm;
                      border: 0.9pt solid #2c3440; border-radius: 0.6mm; }
.cover { padding-top: 18mm; }
.cover .kicker { text-transform: uppercase; letter-spacing: 2pt; font-size: 9.5pt; color: #6b7280; margin-bottom: 3mm; }
.cover .subtitle { font-size: 15pt; color: #2c3440; margin-bottom: 10mm; }
.cover .meta { font-size: 9pt; color: #6b7280; margin-top: 6mm; }
ol.toc { font-size: 11.5pt; line-height: 1.9; }
"""


def find_browser():
    for candidate in BROWSERS:
        path = candidate if os.path.isabs(candidate) else shutil.which(candidate)
        if path and os.path.exists(path):
            return path
    return None


def main():
    if not os.path.isdir(CAPTURES):
        sys.exit("no captures: run scripts/manual_tests/capture_tutorial.py first")
    shutil.rmtree(OUT, ignore_errors=True)
    os.makedirs(IMAGES)
    document = (
        '<!doctype html><html lang="fr"><head><meta charset="utf-8">'
        f"<title>Patine — tutoriel M0 à M3</title><style>{STYLE}</style></head>"
        f"<body>{body()}</body></html>"
    )
    page = os.path.join(OUT, "index.html")
    with open(page, "w", encoding="utf-8") as file:
        file.write(document)
    print(f"wrote {page}")

    browser = find_browser()
    if browser is None:
        sys.exit("Edge or Chrome not found: open the HTML page and print it to PDF by hand")
    url = "file:///" + os.path.abspath(page).replace(os.sep, "/")
    pdf = os.path.abspath(PDF)
    subprocess.run(
        [browser, "--headless", "--disable-gpu", "--no-pdf-header-footer",
         f"--print-to-pdf={pdf}", url],
        check=True, capture_output=True, timeout=180,
    )
    print(f"wrote {PDF} ({os.path.getsize(pdf) // 1024} KiB)")


if __name__ == "__main__":
    main()
