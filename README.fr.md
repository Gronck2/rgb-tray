# RGB Tray

[English](README.md) · [Русский](README.ru.md) · [简体中文](README.zh-CN.md) · [Español](README.es.md) · [Deutsch](README.de.md) · **Français** · [Português (Brasil)](README.pt-BR.md) · [日本語](README.ja.md)

Une petite application Windows pour la zone de notification qui pilote l'éclairage RGB des claviers, souris et lampes directement via HID (et les ampoules Tuya via le réseau local), sans logiciel constructeur (G HUB, Synapse, Armoury Crate, SignalRGB) tournant en arrière-plan.

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="Panneau de la zone de notification">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="Éditeur touche par touche">
</p>

## Périphériques pris en charge

| Périphérique | Connexion | Ce qui fonctionne |
|---|---|---|
| Logitech G915 TKL | Récepteur LIGHTSPEED | Statique, respiration, spectre, éteint ; couleurs touche par touche ; batterie |
| Razer Naga V2 Pro | Dongle HyperSpeed / station d'accueil | Zones du logo et de la molette séparées ; effets ; batterie |
| Razer Mouse Dock Pro | USB | Statique, respiration, spectre, éteint |
| Environ 90 autres souris, claviers et accessoires Razer (Basilisk, DeathAdder, Viper, Cobra, BlackWidow, Huntsman, Ornata, Firefly, Goliathus, Base Station…), d'après les données d'OpenRazer — [liste](profiles) | USB / dongle | Statique, respiration, spectre, éteint ; zones des souris ; batterie. **Non testé** : la carte l'indique, et tout retour est le bienvenu. Les claviers sans fil HyperSpeed ne fonctionnent pour l'instant qu'en filaire. |
| Clavier de portable ASUS (Aura) | Intégré | Statique, respiration, spectre, éteint ; zones du clavier et lampes du châssis via LampArray |
| Geekboard MacroPad v2 (QMK/VIA) | USB | Statique, respiration, spectre, éteint |
| Autres claviers QMK avec VIA et RGB (Keychron, claviers personnalisés…) | USB | Statique, respiration, spectre, éteint, luminosité (respiration et spectre dessinés par l'application) |
| Périphériques conformes au standard Windows Éclairage dynamique (HID LampArray) | USB / intégré | Statique, respiration, spectre, éteint, zones fournies par le périphérique (respiration et spectre dessinés par l'application) |
| Ampoules Wi-Fi Tuya (Smart Life), par ex. LINUS A60 | Wi-Fi, réseau local | Couleur, blanc (une couleur proche du blanc sélectionne les LED blanches et leur teinte), respiration, spectre, éteint ; ne s'éteint pas avec l'écran. Nécessite la clé locale de l'ampoule, voir ci-dessous |

D'autres modèles utilisant les mêmes protocoles peuvent fonctionner, mais n'ont pas été testés.

### Ampoules Tuya

RGB Tray pilote une ampoule Tuya via le réseau local (protocole 3.3) grâce à la *clé locale* de l'ampoule, que seul le cloud Tuya délivre. Récupérez-la une fois pour toutes, par exemple avec [tinytuya](https://github.com/jasonacox/tinytuya) (`python -m tinytuya wizard`) ou via la connexion par QR code Smart Life de `tuya-device-sharing-sdk`. Listez ensuite les ampoules dans `%APPDATA%\rgbtray\tuya.toml` :

```toml
[[device]]
name = "Lampe de bureau"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

L'application trouve d'elle-même l'ampoule sur le réseau local et mémorise son adresse (`local_ip`). L'ampoule doit être enregistrée dans **Smart Life** ou **Tuya Smart** : les comptes des déclinaisons de marque de l'application ne peuvent pas fournir de clés. Une ampoule n'accepte qu'une connexion locale à la fois : tant que l'application Smart Life est ouverte sur un téléphone, RGB Tray ne peut pas la joindre.

### Demander la prise en charge d'un nouveau périphérique

Clic droit sur l'icône de la zone de notification → **Request device support…** (demander la prise en charge d'un périphérique). RGB Tray liste vos périphériques HID, signale ceux dont l'éclairage est peut-être pilotable, copie le rapport dans le presse-papiers et ouvre le [formulaire de demande](../../issues/new?template=device-request.yml). Indiquez le modèle et collez le rapport. Les numéros de série y sont tronqués.

Un périphérique Razer ou QMK/VIA, ou un périphérique au protocole simple, peut souvent être ajouté tout de suite, sans nouvelle version : il suffit de le décrire dans un petit fichier de profil ([docs/profiles.md](docs/profiles.md)).

### Ajouter votre périphérique avec Claude

Avec [Claude Code](https://claude.com/claude-code), vous pouvez faire fonctionner un périphérique vous-même, sans connaître Rust ni HID. Clonez ce dépôt, ouvrez-y Claude Code et tapez `/add-device`. Claude lit le rapport du périphérique, cherche le protocole (OpenRGB, OpenRazer, une capture USB du logiciel constructeur), écrit un profil ou un pilote et le teste avec vous : comme il ne voit pas l'éclairage, il vous demande après chaque étape ce qu'a fait le périphérique. Les commandes susceptibles d'endommager un périphérique (mises à jour du firmware, écriture en mémoire flash) sont exclues. Une fois que tout fonctionne, Claude peut le soumettre sous forme de pull request pour qu'il soit intégré pour tout le monde.

## Fonctionnalités

L'interface de l'application est disponible en anglais et en russe uniquement.

- Un seul panneau pour tous les périphériques : couleur, effet, luminosité, pour tous à la fois ou périphérique par périphérique.
- Une luminosité globale, à laquelle s'ajoute une luminosité propre à chaque périphérique.
- Vitesse de la respiration et du spectre, globale et par périphérique.
- Les effets intégrés au firmware d'un clavier sont proposés dans son éditeur de touches, à côté des préréglages : vague et ondulation sur le Logitech G915. Peindre une touche rétablit les couleurs des touches.
- Éditeur touche par touche pour le clavier Logitech : pinceau, pipette, arc-en-ciel, dégradé bicolore, préréglages WASD et lettres seules, mise en surbrillance d'un mot, schémas enregistrés, dispositions ANSI/ISO.
- Préréglages avec raccourcis clavier globaux (y compris F13–F24 avec Ctrl/Alt/Shift/Win).
- Un raccourci global par périphérique qui l'éteint puis le rallume dans son état précédent (le bouton clavier après le nom sur sa carte : F13–F24 ou n'importe quel raccourci), par exemple pour une lampe de pièce. Les préréglages ne sont pas affectés.
- Plusieurs périphériques identiques avec des réglages distincts ; les périphériques sont reconnus par leur numéro de série.
- Les périphériques déconnectés sont conservés dans une archive avec leurs réglages et réapparaissent lorsqu'ils sont reconnectés.
- Statut « No link » (pas de liaison) lorsqu'un périphérique sans fil est en veille ou éteint ; les couleurs sont réappliquées à son réveil.
- Réapplique l'éclairage après la mise en veille (y compris Modern Standby) et lors du branchement de périphériques.
- Mises à jour : une fois par jour, l'application interroge GitHub sur la dernière version ; si une version plus récente existe, une carte apparaît sur le panneau et l'installe en un clic. La vérification peut être désactivée dans le menu de la zone de notification.
- Suit l'écran : l'éclairage baisse quand Windows assombrit l'écran et s'éteint tant que l'écran est éteint (inactivité, verrouillage, veille). Désactivable dans le menu de la zone de notification.
- Alertes de batterie faible à 10 % et 5 %.
- L'icône de la zone de notification affiche la couleur actuelle.
- Mode épinglé : le panneau reste ouvert au premier plan et peut être déplacé par son en-tête.
- Export et import de tous les réglages dans un fichier. Les préréglages peuvent être partagés un par un : clic droit sur un préréglage → Save to a file… (enregistrer dans un fichier), et le destinataire le charge avec Load settings or a preset… (charger des réglages ou un préréglage) dans le menu de la zone de notification ; il s'applique à ses périphériques des mêmes modèles (à tous, s'il en a plusieurs), et l'application indique quels modèles du fichier il ne possède pas.
- Interface en anglais et en russe : en russe lorsque Windows ou l'une de ses dispositions de clavier est en russe, en anglais sinon ; modifiable dans le menu de la zone de notification.
- Thème clair et sombre suivant celui de Windows.

## Installation

1. Téléchargez `RGBTray-Setup-<version>.exe` depuis la page [Releases](../../releases).
2. Lancez-le. L'application s'installe dans `%LOCALAPPDATA%\Programs\RGB Tray` pour l'utilisateur actuel, sans droits d'administrateur, puis démarre.
3. Elle peut être désinstallée depuis Paramètres Windows → Applications.

Le programme d'installation n'est pas encore signé, Windows SmartScreen peut donc afficher un avertissement (Informations complémentaires → Exécuter quand même). Les réglages sont stockés dans `%APPDATA%\rgbtray` et sont conservés lors des mises à jour et réinstallations. Le dossier `backups` qui s'y trouve garde des copies des réglages des 14 derniers jours et d'avant chaque mise à jour ; si le fichier de réglages est endommagé, l'application reprend d'elle-même la dernière copie valide.

Fermez les logiciels d'éclairage des constructeurs avant d'utiliser RGB Tray : deux programmes qui écrivent sur le même périphérique se contrarient. Sous Windows 11, le service « Logitech LampArray » (Éclairage dynamique) fait ignorer les effets au G915 ; l'application vous en avertit et propose de le désactiver.

## Ligne de commande

| Commande | Rôle |
|---|---|
| `rgbtray --diagnose [file.txt]` | Écrit le rapport des périphériques : versions de l'application et de Windows, périphériques pris en charge et tous les périphériques HID regroupés avec leurs interfaces, les contrôleurs d'éclairage probables en premier. Sans nom de fichier, il est enregistré dans `%APPDATA%\rgbtray\rgbtray-diagnose.txt` et ouvert dans le Bloc-notes. Également disponible dans le menu de la zone de notification. |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | Envoie des rapports HID bruts et ajoute les réponses au fichier, pour décortiquer le protocole d'un périphérique ([docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)). |
| `rgbtray --profile-check <out.txt> <profile.toml>` | Lit un fichier de profil sans toucher à aucun périphérique et écrit ce que l'application comprend de chaque modèle, ou ce qui ne va pas. |
| `rgbtray --profile-test <out.txt> <profile.toml>` | Joue des couleurs et des effets sur les périphériques d'un profil `[hid]` pas encore installé et consigne chaque étape. |
| `rgbtray --show` | Démarre avec le panneau ouvert et épinglé. |
| `rgbtray --uninstall` | Désinstalle l'application (les réglages sont conservés). |

## Compiler depuis les sources

Nécessite Windows 10/11 et [Rust](https://rustup.rs) stable, avec la chaîne d'outils `x86_64-pc-windows-gnu` ou `-msvc`. Après avoir installé rustup, ouvrez un nouveau terminal pour que `cargo` soit dans le PATH.

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

Le programme d'installation est le même exécutable : nommé `RGBTray-Setup-*.exe` (ou lancé avec `--install`), il s'installe lui-même.

## Contribuer

Les rapports de périphériques, les tests sur votre matériel, les signalements de bugs et les pull requests sont les bienvenus. [CONTRIBUTING.md](CONTRIBUTING.md) explique comment compiler et vérifier l'application, comment ajouter un périphérique et ce qu'une pull request doit contenir.

## Remerciements

Les protocoles des périphériques ont été appris grâce à ces projets ; RGB Tray ne contient aucun code issu de ceux-ci :

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) — éclairage Logitech, Razer et ASUS Aura
- [OpenRazer](https://github.com/openrazer/openrazer) — protocole Razer
- [Solaar](https://github.com/pwr-Solaar/Solaar) — Logitech HID++
- [QMK](https://qmk.fm) et [VIA](https://www.caniusevia.com) — protocole d'éclairage du MacroPad

## Licence

Distribué sous la licence [MIT](LICENSE-MIT) ou [Apache-2.0](LICENSE-APACHE), au choix.

Sauf mention contraire explicite de votre part, toute contribution que vous soumettez intentionnellement en vue de son inclusion dans ce projet, au sens de la licence Apache-2.0, sera soumise à la double licence ci-dessus, sans conditions ni modalités supplémentaires.

Sans lien avec Logitech, Razer, ASUS, Geekboard ou Tuya. Les marques citées appartiennent à leurs propriétaires respectifs.

Ceci est une traduction : en cas de divergence, le README en anglais fait foi.
