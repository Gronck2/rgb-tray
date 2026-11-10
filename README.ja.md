# RGB Tray

[English](README.md) · [Русский](README.ru.md) · [简体中文](README.zh-CN.md) · [Español](README.es.md) · [Deutsch](README.de.md) · [Français](README.fr.md) · [Português (Brasil)](README.pt-BR.md) · **日本語**

**ウェブサイト：** [rgb-tray.gronck.com](https://rgb-tray.gronck.com/ja/)

キーボード、マウス、ライトの RGB ライティングを HID で直接制御する（Tuya 電球はローカルネットワーク経由）、Windows 用の小さなトレイアプリです。メーカー製ソフトウェア（G HUB、Synapse、Armoury Crate、SignalRGB）をバックグラウンドで動かしておく必要はありません。

**お使いのデバイスが一覧にない場合は、自分で追加できます。** このリポジトリを [Claude Code](https://claude.com/claude-code) で開き、`/add-device` と入力してください。Claude がプロトコルを調べ、プロファイルまたはドライバーを書き、あなたと一緒にテストします。Rust や HID の知識は不要です（[仕組み](#claude-でデバイスを追加する)）。

<p>
  <img src="docs/screenshots/panel-dark.png" width="300" alt="トレイパネル">
  <img src="docs/screenshots/editor-dark.png" width="520" alt="キーごとのエディター">
</p>

## 対応デバイス

| デバイス | 接続 | 対応機能 |
|---|---|---|
| Logitech G915 TKL | LIGHTSPEED レシーバー | 固定色、ブリージング、スペクトラム、消灯、キーごとの色、バッテリー |
| Razer Naga V2 Pro | HyperSpeed ドングル / ドック | ロゴとスクロールホイールを個別のゾーンとして設定、エフェクト、バッテリー |
| Razer Mouse Dock Pro | USB | 固定色、ブリージング、スペクトラム、消灯 |
| OpenRazer のデータに基づく、その他約 90 種の Razer マウス・キーボード・アクセサリー（Basilisk、DeathAdder、Viper、Cobra、BlackWidow、Huntsman、Ornata、Firefly、Goliathus、Base Station など）— [一覧](profiles) | USB / ドングル | 固定色、ブリージング、スペクトラム、消灯、マウスのゾーン、バッテリー。**未検証**：カードにその旨が表示されます。動作報告を歓迎します。ワイヤレスの HyperSpeed キーボードは現在のところ有線接続時のみ対応。 |
| ASUS ノート PC のキーボード（Aura） | 内蔵 | 固定色、ブリージング、スペクトラム、消灯、LampArray によるキーボードのゾーンと本体のランプ |
| Geekboard MacroPad v2（QMK/VIA） | USB | 固定色、ブリージング、スペクトラム、消灯 |
| VIA と RGB に対応したその他の QMK キーボード（Keychron、自作キーボードなど） | USB | 固定色、ブリージング、スペクトラム、消灯、明るさ（ブリージングとスペクトラムはアプリ側で描画） |
| Windows のダイナミック ライティング規格（HID LampArray）に対応したデバイス | USB / 内蔵 | 固定色、ブリージング、スペクトラム、消灯、デバイスが持つゾーン（ブリージングとスペクトラムはアプリ側で描画） |
| Tuya の Wi-Fi 電球（Smart Life）、例：LINUS A60 | Wi-Fi、ローカルネットワーク | カラー、白色（白に近い色を選ぶと白色 LED とその色味が使われます）、ブリージング、スペクトラム、消灯。ディスプレイと連動して消灯はしません。電球のローカルキーが必要です（下記参照） |

同じプロトコルを使う他のモデルも動作する可能性がありますが、未検証です。

### Tuya 電球

RGB Tray は、Tuya のクラウドからしか入手できない電球の*ローカルキー*を使い、ローカルネットワーク経由（プロトコル 3.3）で Tuya 電球を制御します。キーは一度取得すれば十分です。たとえば [tinytuya](https://github.com/jasonacox/tinytuya)（`python -m tinytuya wizard`）や、`tuya-device-sharing-sdk` の Smart Life QR ログインで取得できます。取得したら、`%APPDATA%\rgbtray\tuya.toml` に電球を記載します。

```toml
[[device]]
name = "デスクライト"
id = "bf1234567890abcdef12"
local_key = "0123456789abcdef"
```

アプリはローカルネットワーク上の電球を自動で見つけ、そのアドレス（`local_ip`）を記憶します。電球は **Smart Life** または **Tuya Smart** に登録されている必要があります。ブランド名を変えた同系アプリのアカウントではキーを取得できません。電球は同時に 1 つのローカル接続しか受け付けないため、スマートフォンで Smart Life アプリを開いている間は RGB Tray から接続できません。

### 新しいデバイスのリクエスト

トレイアイコンを右クリック → **Request device support…**（デバイス対応をリクエスト）。RGB Tray が HID デバイスを一覧にし、制御可能なライティングを持つ可能性のあるものに印を付け、レポートをクリップボードにコピーして[リクエストフォーム](../../issues/new?template=device-request.yml)を開きます。モデル名を入力し、レポートを貼り付けてください。レポート内のシリアル番号は短縮されます。

Razer や QMK/VIA のデバイス、あるいは単純なプロトコルのデバイスであれば、新しいバージョンを待たずにすぐ追加できることがよくあります。小さなプロファイルファイルにデバイスの情報を記述してください（[docs/profiles.md](docs/profiles.md)）。

### Claude でデバイスを追加する

[Claude Code](https://claude.com/claude-code) を使えば、Rust や HID の知識がなくても自分でデバイスを動かせます。このリポジトリをクローンし、その中で Claude Code を開いて `/add-device` と入力してください。Claude はデバイスレポートを読み、プロトコルを調べ（OpenRGB、OpenRazer、メーカー製アプリの USB キャプチャなど）、プロファイルまたはドライバーを書き、あなたと一緒にテストします。Claude にはライトが見えないため、各ステップのあとでデバイスがどう反応したかを尋ねます。デバイスを損傷させるおそれのあるコマンド（ファームウェア更新、フラッシュへの保存）は使用しません。動作したら、Claude がプルリクエストとして送信し、全員が使える組み込みのデバイスにすることもできます。

## 機能

アプリのインターフェースは英語とロシア語のみです。

- すべてのデバイスを 1 つのパネルで操作：色、エフェクト、明るさを全デバイス一括またはデバイスごとに設定。
- 全体の明るさに加えて、その上にデバイスごとの明るさを設定可能。
- ブリージングとスペクトラムの速度を、全体およびデバイスごとに設定可能。
- キーボードのファームウェアエフェクトを、キーエディターのプリセットの隣に表示：Logitech G915 ではウェーブとリップル。キーを塗るとキーごとの色に戻ります。
- Logitech キーボード用のキーごとのエディター：ブラシ、スポイト、レインボー、2 色グラデーション、WASD のみ・文字キーのみのプリセット、単語のハイライト、スキームの保存、ANSI/ISO 配列。
- グローバルホットキー付きのプリセット（Ctrl/Alt/Shift/Win と組み合わせた F13–F24 を含む）。
- デバイスごとのグローバルホットキーで、そのデバイスを消灯し、元の状態で再点灯（カードの名前の後ろにあるキーボードボタンで設定、F13–F24 または任意のショートカット）。たとえば部屋のランプに便利です。プリセットには影響しません。
- 同じデバイスを複数台つないでも個別に設定可能。デバイスはシリアル番号で識別されます。
- 切断されたデバイスは設定ごとアーカイブに保管され、再接続すると元に戻ります。
- ワイヤレスデバイスがスリープ中または電源オフのときは「No link」（接続なし）と表示。復帰すると色が再適用されます。
- スリープ（モダン スタンバイを含む）からの復帰時や、デバイスの接続時にライティングを再適用。
- アップデート：1 日 1 回 GitHub に最新リリースを問い合わせ、新しいバージョンがあればパネルにカードを表示し、ワンクリックでインストールできます。チェックはトレイメニューでオフにできます。
- ディスプレイに連動：Windows が画面を暗くするとライティングも暗くなり、画面がオフの間（アイドル、ロック、スリープ）は消灯します。トレイメニューでオフにできます。
- バッテリー残量 10% と 5% で低バッテリー警告。
- トレイアイコンが現在の色を表示。
- ピン留めモード：パネルが最前面に開いたままになり、ヘッダーをドラッグして移動できます。
- すべての設定をファイルにエクスポート／インポート可能。プリセットは 1 つずつ共有できます：プリセットを右クリック → Save to a file…（ファイルに保存）。受け取った側はトレイメニューの Load settings or a preset…（設定またはプリセットを読み込む）で読み込みます。プリセットは受け取った側の同じモデルのデバイス（複数台あればすべて）に適用され、ファイル内のどのモデルが手元にないかもアプリが知らせます。
- 英語とロシア語のインターフェース：Windows またはそのキーボードレイアウトのいずれかがロシア語ならロシア語、それ以外は英語。トレイメニューで切り替え可能。
- Windows に合わせたライト／ダークテーマ。

## インストール

1. [Releases](../../releases) から `RGBTray-Setup-<version>.exe` をダウンロードします。
2. 実行します。アプリは管理者権限なしで、現在のユーザー向けに `%LOCALAPPDATA%\Programs\RGB Tray` へ自身をインストールし、起動します。
3. アンインストールは Windows の設定 → アプリから行えます。

インストーラーにはまだコード署名がないため、Windows SmartScreen が警告を表示することがあります（詳細情報 → 実行）。設定は `%APPDATA%\rgbtray` に保存され、アップデートや再インストール後も保持されます。そこにある `backups` フォルダーには、直近 14 日間と各アップデート前の設定のコピーが保管されます。設定ファイルが壊れた場合は、アプリが正常に読める最新のコピーを自動的に使用します。

RGB Tray を使う前に、メーカー製のライティングソフトウェアを終了してください。2 つのプログラムが同じデバイスに書き込むと、互いに競合します。Windows 11 では「Logitech LampArray」サービス（ダイナミック ライティング）によって G915 がエフェクトを無視するようになります。アプリはこれを警告し、無効化を提案します。

## コマンドライン

| コマンド | 動作 |
|---|---|
| `rgbtray --diagnose [file.txt]` | デバイスレポートを書き出します：アプリと Windows のバージョン、対応デバイス、インターフェースごとにまとめたすべての HID デバイス（ライティングコントローラーの可能性が高いものが先頭）。ファイル名を省略すると `%APPDATA%\rgbtray\rgbtray-diagnose.txt` に保存され、メモ帳で開きます。トレイメニューからも実行できます。 |
| `rgbtray --hid-send <out.txt> <vid:pid[:page[:usage]]> <feature|output|get> "<hex>"…` | 生の HID レポートを送信し、応答をファイルに追記します。デバイスのプロトコルを解析するためのものです（[docs/profiles.md](docs/profiles.md#hid-any-device-with-a-simple-protocol)）。 |
| `rgbtray --profile-check <out.txt> <profile.toml>` | デバイスに一切触れずにプロファイルファイルを読み込み、各モデルがアプリにどう解釈されるか、または何が間違っているかを書き出します。 |
| `rgbtray --profile-test <out.txt> <profile.toml>` | まだインストールしていない `[hid]` プロファイルのデバイスで色とエフェクトを再生し、各ステップを記録します。 |
| `rgbtray --show` | パネルを開き、ピン留めした状態で起動します。 |
| `rgbtray --uninstall` | アプリを削除します（設定は保持されます）。 |

## ソースからのビルド

Windows 10/11 と stable 版の [Rust](https://rustup.rs)（`x86_64-pc-windows-gnu` または `-msvc` ツールチェーン）が必要です。rustup をインストールしたら、`cargo` が PATH に入るように新しいターミナルを開いてください。

```powershell
cargo build --release            # target\release\rgbtray.exe
.\scripts\release.ps1            # installer in dist\RGBTray-Setup-<version>.exe
.\scripts\release.ps1 -Install   # build and install/update silently
```

インストーラーは同じ実行ファイルです。`RGBTray-Setup-*.exe` という名前にする（または `--install` を付けて起動する）と、自身をインストールします。

## コントリビューション

デバイスレポート、お手持ちのハードウェアでのテスト、バグ報告、プルリクエストを歓迎します。[CONTRIBUTING.md](CONTRIBUTING.md) に、アプリのビルドと確認の方法、デバイスの追加方法、プルリクエストに必要なものが説明されています。

## 謝辞

デバイスのプロトコルは以下のプロジェクトから学びました。RGB Tray にはこれらのプロジェクトのコードは含まれていません。

- [OpenRGB](https://gitlab.com/CalcProgrammer1/OpenRGB) — Logitech、Razer、ASUS Aura のライティング
- [OpenRazer](https://github.com/openrazer/openrazer) — Razer のプロトコル
- [Solaar](https://github.com/pwr-Solaar/Solaar) — Logitech HID++
- [QMK](https://qmk.fm) と [VIA](https://www.caniusevia.com) — MacroPad のライティングプロトコル

## ライセンス

[MIT](LICENSE-MIT) または [Apache-2.0](LICENSE-APACHE) のいずれか（お好みで選択）の下でライセンスされます。

明示的に別段の意思表示をしない限り、Apache-2.0 ライセンスの定義に従い、本プロジェクトへの取り込みを意図してあなたが提出したすべてのコントリビューションは、追加の条項や条件なしに、上記のとおりデュアルライセンスされるものとします。

Logitech、Razer、ASUS、Geekboard、Tuya とは一切関係ありません。商標はそれぞれの所有者に帰属します。

これは翻訳です。内容に相違がある場合は、英語版の README が優先されます。
