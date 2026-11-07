# YuiLauncher

Yuiアプリの起動とアップデートをまとめて行うWindows向けランチャーです。

## 開発

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

`npm run test:ui`は`tests/fixture.cjs`の偽データで動くため、GitHubや実際のアプリには触れません。

## アイコン

`assets/icon-source.webp`（正方形）を差し替えて`npm run icon`を実行すると、`assets/icon.ico`と`assets/icon.png`を作り直します。

## アプリの追加

`electron/catalog.cjs`に1件追加し、アイコンを`assets/apps`に置きます。対象アプリのGitHub Releasesには次が必要です。

- Windows用インストーラー（`installerPattern`に一致する名前）
- インストーラーのSHA-256を記載したチェックサムファイル（`checksumPattern`に一致する名前）
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

## YuiLauncher自身の更新

起動時と6時間ごと、「更新を確認」を押したときに確認します。失敗すると画面に表示し、30分後に再試行します。

詳しい経過は`%APPDATA%\yui-launcher\logs\self-update.log`に残ります。1MBを超えると`self-update.log.old`に切り替わります。

## リリース

バージョン番号は「年.月日.その日の連番」です。例：`2026.1107.1`は2026年11月7日の1版目です。月日の先頭のゼロは省きます（1月3日なら`103`）。

```bash
npm version 2026.1107.1 --no-git-tag-version
```

```bash
npm run dist
```

`release`にインストーラーと`latest.yml`が作られます。ランチャー自身の更新には、`suzunayui/YuiLauncher`のGitHub Releaseに、インストーラー、`.blockmap`、`latest.yml`を公開します。
