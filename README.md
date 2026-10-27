# Osechi

雙峰祭のために開発中の映像スイッチング/伝送ソフトウエア

## コンセプト
Osechiは、一般的なスペックのPCでも安定して動作する、軽量で扱いやすい映像スイッチング・伝送ソフトウエアを目指しています。

- **安定性と軽量さ**: 従来のOBSの代替となる安定性を確保しつつ、一般的なPC環境でも軽快に動作します。
- **手軽なLAN内伝送**: 独自の軽量な通信プロトコルを実装し、ローカルネットワーク内で手軽かつ低遅延な映像伝送を実現します。
- **マルチプラットフォーム**: 特定のOSに依存しない設計により、幅広い環境で柔軟に運用できます。

## 入力

- 映像: Webカメラ、画面キャプチャ、NDI、RTMP
- 音声: OSの入力デバイス、AES67、NDI、RTMP

NDI は NDI Runtime がインストールされていれば自動検出します。RTMP 入力は起動時に `0.0.0.0:1935` で待ち受け、OBS などから `rtmp://<ホスト>:1935/live/<キー>` へ配信すると映像・音声がソース一覧に現れます。待ち受け先は `--rtmp-ingest 0.0.0.0:1935` で変更でき、`--no-rtmp-ingest` で無効化できます。

### Osechi同士でRTMPを試す

RTMP出力は1080pの変換とエンコードが重いため、確認にはreleaseビルドを使ってください。debugビルドでは出力が数fpsになり、カクついて見えることがあります。

受信側を先に、別のターミナルで起動します。

```powershell
cargo build --release
.\target\release\osechi.exe
```

送信側は、さらに別のターミナルでRTMP入力とAPIポートを分けて起動します。

```powershell
$env:OSECHI_API_PORT = "7879"
.\target\release\osechi.exe --no-rtmp-ingest --rtmp-url rtmp://127.0.0.1:1935/live/test
```

送信側で映像をProgramに割り当ててOutputのStartを押すと、受信側のSOURCESに `RTMP live/test` と `RTMP live/test (Audio)` が現れます。別PCから送る場合は `127.0.0.1` を受信側のIPに変えてください。

画面キャプチャは仕様上約15fpsなので、滑らかさを確認する場合はWebカメラを使ってください。

## 外部API

donguriなど外部システムからマスターミュートを操作するためのHTTP APIを、起動時にポート`7878`(環境変数`OSECHI_API_PORT`で変更可能)で提供します。OpenAPIスキーマ・Swagger UIも同ポートから参照できます。詳しくは[ミュートAPI](docs/mute-api.md)を参照してください。