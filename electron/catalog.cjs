'use strict';

// Apps managed by the launcher. Add an entry here to manage another app.
// registryKey is the app's key under ...\CurrentVersion\Uninstall.
module.exports = [
  {
    id: 'yuitracking',
    name: 'YuiTracking',
    description: 'VRMアバターのトラッキングと仮想カメラ',
    repo: 'suzunayui/YuiTracking',
    registryKey: '{A4A68FE9-BAEA-4CBB-A7DD-877B688187BD}_is1',
    exeName: 'YuiTracking.exe',
    installerPattern: /^YuiTracking-Setup-[\d.]+-x64\.exe$/,
    checksumPattern: /^SHA256SUMS.*\.txt$/,
    silentArgs: ['/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART'],
    icon: '../assets/apps/YuiTracking.ico',
  },
  {
    id: 'yuivoicechanger',
    name: 'YuiVoiceChanger',
    description: 'リアルタイムボイスチェンジャー',
    repo: 'suzunayui/YuiVoiceChanger',
    registryKey: '190c2e46-466f-54a6-8fd8-f3728d1d9996',
    exeName: 'YuiVoiceChanger.exe',
    installerPattern: /^YuiVoiceChanger-Setup-[\d.]+\.exe$/,
    checksumPattern: /^SHA256SUMS.*\.txt$/,
    silentArgs: ['/S'],
    icon: '../assets/apps/YuiVoiceChanger.ico',
  },
  {
    id: 'yuilivekit',
    name: 'YuiLiveKit',
    description: 'OBS向けのコメント表示と音声字幕',
    repo: 'suzunayui/YuiLiveKit',
    registryKey: '54a05e14-5bce-5486-8d4f-9dc5271d69b3',
    exeName: 'YuiLiveKit.exe',
    installerPattern: /^YuiLiveKit-Setup-[\d.]+\.exe$/,
    checksumPattern: /^SHA256SUMS.*\.txt$/,
    silentArgs: ['/S'],
    icon: '../assets/apps/YuiLiveKit.ico',
  },
];
