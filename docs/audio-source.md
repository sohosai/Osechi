# 音声ソース（Audio Source）

音声ソースの検出・管理・ストリーミングに関わる型の設計ドキュメント。

## モジュール構成

```
source/audio/
├── mod.rs           型定義（SourceId, AudioChunk, SourceKind, Descriptor, Stream）
├── manager.rs       SourceManager（ソース一覧の保持と open）
└── input_device.rs  InputDeviceStream（cpal による OS 入力デバイスの Stream 実装）
```

## 型一覧

### `SourceId`

音声ソースを一意に識別する enum。

```rust
pub enum SourceId {
    InputDevice(String),
    // 将来: SystemAudio(...), File(...), ...
}
```

- `Display` を実装し、ログや内部キー生成に使える文字列表現を取得できる
- `Clone`, `Hash`, `Eq` を実装しており、`HashMap` のキーとして利用可能

### `AudioChunk`

音声の一定時間分のデータを表す。サンプルデータは `Arc<Vec<f32>>` で保持され、複数スレッド間で共有できる。

```rust
pub struct AudioChunk {
    pub samples: Arc<Vec<f32>>, // interleaved PCM
    pub sample_rate: u32,
    pub channels: u16,
    pub frames: usize,
    pub pts: u64,
}
```

- `samples` は `f32` の interleaved PCM に統一している
- `frames` はチャンネル単位ではなく、音声フレーム数を表す
- `pts` はストリーム開始からの累積フレーム数を表す
- `AudioChunk` の `frames` 数も可変

### `SourceKind`

ソース種別ごとの接続パラメータを保持する enum。`Descriptor` のフィールドとして使われる。

```rust
pub enum SourceKind {
    InputDevice {
        device_id: cpal::DeviceId,
        name: String,
    },
    // 将来: SystemAudio { ... }, File { ... }, ...
}
```

### `Descriptor`

音声ソースの設計図。ストリームを開かなくても取得できるメタ情報（ID・名前・種別）を保持する。

```rust
pub struct Descriptor {
    pub id: SourceId,
    pub name: String,
    pub kind: SourceKind,
}
```

- `Clone` 可能で軽量。UI での一覧表示や選択状態の保持に使う
- `open()` を呼ぶと、`SourceKind` に応じた具体的な `Stream` を生成して返す

### `Stream`

ハードウェアデバイスへの接続を保持し、音声チャンクを取得するためのtrait。

```rust
pub trait Stream: Send {
    fn get_chunk(&self) -> Result<Option<AudioChunk>, AppError>;
}
```

- `Ok(Some(chunk))` — 新しい音声データが利用可能
- `Ok(None)` — まだ届いていない（ノンブロッキング）
- `Err(e)` — ストリームエラー
- drop 時にハードウェア接続が自動切断される

## バッファ方針

ライブ配信用途では、遅延が積み上がるよりも最新の音声に追いつくことを優先する。

そのため、音声 callback から受け取った `AudioChunk` は固定長リングバッファに入れる。バッファが満杯の場合は古い chunk を捨て、新しい chunk を保存する。

```text
callback -> ring buffer<AudioChunk> -> get_chunk()
```

リングバッファには `AudioChunk` のみを入れる。`AppError` は音声データではなく制御状態なので、同じバッファには混ぜない。

```rust
ring_buffer: RingBuffer<AudioChunk>
last_error: Option<AppError>
```

この分離により、バッファ満杯時の drop ポリシーを「古いメディアデータを捨てる」に限定できる。エラーを同じリングバッファに入れると、古いエラーを捨ててよいのか、エラーと chunk の順序に意味があるのかが曖昧になるため避ける。

## フレームサイズ方針

`cpal` の callback に届く `data` の長さは OS・ドライバ・負荷によって変動する可能性がある。`AudioSource` はこの可変サイズを固定化しない。

`AudioSource` の責務は、デバイスから届いた時系列の PCM chunk を、サンプル形式を揃えて渡すことに限定する。エンコーダが固定フレームサイズを要求する場合は、エンコード直前、または mixer/audio pipeline 側で固定サイズに組み直す。

## SourceManager

`SourceManager` は検出結果の保持と `SourceId` からの `open()` を担当する。各 backend 固有の scan 処理は、それぞれの backend モジュールに置く。

```rust
pub fn input_device_scan(&mut self) {
    self.input_devices = input_device::scan();
}
```

これにより、`manager.rs` は cpal などの実装詳細を知らずに済む。将来 backend が増えた場合も、各 backend の scan 実装を個別モジュールに閉じ込められる。
