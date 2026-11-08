# YuiLauncher

Yuiアプリの起動とアップデートをまとめて行う、WindowsとMac（Apple Silicon）向けのランチャーです。

## 開発

Rustが必要です。Tauriで動きます。

```bash
npm install
npm start
```

```bash
npm test
```

```bash
npm run test:ui
```

`npm run test:ui`はデバッグビルドの画面を`src-tauri/src/fixture.rs`の偽データで動かすため、GitHubや実際のアプリには触れません。

## アイコン

`assets/icon-source.webp`（正方形）を差し替えて`npm run icon`を実行すると、`assets/icon.ico`と`assets/icon.png`を作り直します。

## アプリの追加

`src-tauri/src/catalog.rs`に1件追加し、アイコンを`assets/apps`に置きます。Mac版もあるアプリは`mac`に`.app`の名前と配布ファイルの名前を書きます。`mac`がないアプリはMacでは表示しません。対象アプリのGitHub Releasesには次が必要です。

- Windows用インストーラー（`installer_prefix`、バージョン、`installer_suffix`をつなげた名前）
- Mac版があれば、`.app`を入れた`.zip`か`.dmg`（`mac`の`installer_prefix`、バージョン、`installer_suffix`をつなげた名前）
- インストーラーのSHA-256を記載したチェックサムファイル（`SHA256SUMS`で始まる`.txt`）。WindowsとMacで分けても構いません
- `v2026.1107.1`形式のタグ。`-mac.1`や`-beta`が付くタグは対象外です

## 更新の流れ

1. レジストリからインストール済みのバージョンを読みます
2. GitHub Releasesから、Windows用インストーラーを含む最新の安定版を探します
3. ダウンロードしてSHA-256を検証します
4. アプリが起動中なら確認のうえで閉じます。閉じられなくても強制終了はしません
5. サイレントインストールし、レジストリのバージョンが上がったことを確認します
6. 閉じたアプリを起動し直します

## アンインストールの流れ

1. 残るデータを確認ダイアログで案内します
2. アプリが起動中なら閉じます。閉じられなくても強制終了はしません
3. レジストリに登録されたアンインストーラーをサイレント実行します。インストール先の外にあるものは実行しません
4. レジストリから登録が消えたことを確認します

## Macでの動き

インストール済みかは`/Applications`の`.app`と、その`Info.plist`のバージョンで判断します。インストールは`.zip`か`.dmg`から`.app`を取り出し、隣にコピーしてから入れ替えます。途中で失敗しても古いアプリは残ります。アプリを閉じるときはmacOSの「終了」を送り、強制終了はしません。アンインストールは`.app`をゴミ箱に移します。

ランチャー自身の更新は、自分の`.app`を新しいものと入れ替えてから開き直します。ログは`~/Library/Logs/com.suzunayui.yuilauncher/self-update.log`に残ります。

## YuiLauncher自身の更新

起動時と6時間ごと、「更新を確認」を押したときに確認します。失敗すると画面に表示し、30分後に再試行します。

他のアプリと同じように、ダウンロードしてSHA-256を検証します。インストールは「再起動して更新」を押したとき、または終了するときに行います。

詳しい経過は`%LOCALAPPDATA%\com.suzunayui.yuilauncher\logs\self-update.log`に残ります。1MBを超えると`self-update.log.old`に切り替わります。

## Electron版からの移行

以前のElectron版は`electron-updater`で`latest.yml`を読んで更新します。そのためリリースには`latest.yml`も載せます。Electron版がこのインストーラーを実行すると、`src-tauri/installer-hooks.nsh`がElectron版を削除してから入れ替え、新しいランチャーを起動します。

## リリース

バージョン番号は「年.月日.その日の連番」です。例：`2026.1107.1`は2026年11月7日の1版目です。月日の先頭のゼロは省きます（1月3日なら`103`）。

```bash
npm version 2026.1107.1 --no-git-tag-version
```

先にMac版をMacでビルドします。Node.jsがないMacでは、Windowsで`node scripts/prepare-tauri-frontend.cjs`を実行して作った`src-tauri/frontend`を持っていき、次のようにします。

```bash
cargo install tauri-cli --version "^2" --locked
```

```bash
cargo tauri build -c '{"build":{"beforeBuildCommand":""}}'
```

```bash
scripts/package-mac.sh
```

`release`に`YuiLauncher-<バージョン>-mac-arm64.zip`が作られます。これをWindows側の`release`に置いてから、Windowsでビルドします。

```bash
npm run dist
```

`release`に、インストーラー、Mac版の`.zip`、チェックサム、`latest.yml`が揃います。この4つを同じGitHub Releaseに公開します。

チェックサムは、WindowsとMacの分を必ず1つのファイルにまとめます。2026.1107.12以前のランチャーは、リリースにある最初のチェックサムファイルしか読みません。Mac用を別のファイルにすると、名前順でそちらが先に来るため、Windows版を検証できず更新できなくなります。
