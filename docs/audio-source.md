# 音声ソース（Audio Source）

音声ソースの検出・管理・ストリーミングに関わる型の設計ドキュメント。

`Source` / `Catalog` / `Open` / `Feed` / `Live` など映像と共通の骨格は[映像ソース](video-source.md#共通の型)を参照。
ここでは音声固有の部分を説明する。

## モジュール構成

```
source/audio/
├── mod.rs              Chunk, Kind, scan()
├── device.rs           OSの音声入力デバイス(cpal)
└── aes67/
    ├── mod.rs          AES67 フローの受信(Config, Format, open)
    └── discovery.rs    SAP による AES67 フローの自動検出(Discovery)

net/                    AES67 が使うプロトコル
├── multicast.rs        IPv4マルチキャスト受信ソケット
├── rtp.rs              RTP パケット
├── sap.rs              SAP メッセージ
└── sdp.rs              SDP
```

## 型一覧

### `Chunk`

一定時間分の音声。

```rust
pub struct Chunk {
    pub samples: Vec<f32>, // interleaved PCM(-1.0..=1.0)
    pub sample_rate: u32,
    pub channels: u16,
}
```

- サンプル形式は `f32` の interleaved PCM に統一する(デバイスの i16 / u16 や AES67 の L16 / L24 はここで変換済み)
- サンプルレート・チャンネル数はソースのものをそのまま持つ。48kHz ステレオへの変換はミキサー側で行う
- `samples` の長さは可変で、チャンネル数の倍数とも限らない

### `Kind`

音声ソースの種別と、開くのに必要な情報。`Open` を実装する。

```rust
pub enum Kind {
    Device(cpal::DeviceId), // OSの音声入力デバイス
    Aes67(aes67::Config),   // AES67のRTPマルチキャストフロー
}
```

### `scan()`

接続されている音声入力デバイスを探し、`Origin::Scanned` の `Source` を返す。
AES67 フローはスキャンでは見つからない(手動追加するか `Discovery` で見つける)。

## 各方式

### OSの音声入力デバイス(`device.rs`)

- ID は `mic:<cpal の DeviceId>`
- デバイスの既定の入力設定でストリームを開く。対応サンプル形式は F32 / I16 / U16
- cpal のコールバックで受け取ったデータをそのまま `Chunk` にして送る。ストリームは `Feed::with_guard` で `Feed` に持たせ、drop で止まる

### AES67(`aes67/`)

Dante 機器の AES67 モードなどが RTP マルチキャストで流す非圧縮 PCM を受信する。

```rust
pub struct Config {
    pub addr: Ipv4Addr,     // 宛先のマルチキャストアドレス
    pub port: u16,
    pub payload_type: u8,   // これ以外のRTPペイロードタイプは捨てる
    pub format: Format,     // L16 または L24
    pub channels: u16,
    pub sample_rate: u32,
}
```

- ID は `aes67:<addr>:<port>`。同じ宛先は同じフローとみなす
- 受信スレッドで RTP を解釈し、ペイロードタイプが一致するパケットだけを `Chunk` にする
- マルチキャストの Join はローカルの全 IPv4 インターフェースで行う(WSL・VPN などの仮想アダプタに Join を取られて物理 NIC に届かなくなるのを避けるため)
- 既知の制約: PTP によるクロック同期はしない。ジッタバッファを持たず、パケットの並び替えや欠落の補間もしない

フローは次の3通りで一覧に載る。

| 経路 | Origin | 実装 |
|---|---|---|
| 起動設定(会場の Yamaha DM7 のフロー) | `Manual` | `config.rs` の `DM7`。起動時にミキサーにも追加される |
| UI の「Add AES67 Source」ダイアログ | `Manual` | `ui/sources.rs`。一覧から削除できる |
| SAP の告知 | `Discovered` | `aes67::Discovery` |

### `Discovery`(`aes67/discovery.rs`)

SAP(`224.2.127.254:9875`)の告知をバックグラウンドで受信し、いま告知されている AES67 フローを把握する。

- `Discovery::start()`: 受信を始める。SAP のポートが使えなければ `None`(手動追加は引き続き使える)
- `Discovery::sources()`: 届いた告知を反映し、有効なフローを返す。エンジンスレッドが `App::tick` で毎回呼び、`Catalog::sync(Origin::Discovered, ..)` に渡す
- 削除の告知が来るか、5分間再告知が無ければそのフローを取り除く
- SDP がマルチキャストでない・L16 / L24 以外・チャンネル数0のフローは無視する
- 手動追加したフローと同じ宛先が告知されても、`Catalog::sync` の規則により手動追加の方が残る

## バッファ方針

ライブ配信用途では、遅延が積み上がるよりも最新の音声に追いつくことを優先する。

そのため、取得処理から受け取った `Chunk` は `Feed`(固定容量のキュー)に入れ、満杯の場合は古い chunk を捨てる。

```text
cpal callback / AES67 受信スレッド -> Feed<Chunk> -> Live::drain() -> Mixer::process()(エンジンスレッド)
```

容量は64(`FEED_CAPACITY`)。エンジンスレッドが取り出すのは約 10ms に1回(UI の描画でロックが空くのを待つと延びる)で、
AES67 は ptime=1ms ごとに1チャンク届くため、取り出す間隔に余裕を持たせている。
容量が足りないと取り出す前に上書きされ、音が周期的に欠けてノイズになる。

`Feed` には `Chunk` のみを入れ、エラーは「直近のエラー」として別に持つ。
同じキューに入れると、満杯時に古いエラーを捨ててよいのか、エラーと chunk の順序に意味があるのかが曖昧になるため避ける。

## フレームサイズ方針

cpal のコールバックや AES67 のパケットで届く音声の長さは、OS・ドライバ・送信機器・負荷によって変わる。
`source::audio` はこの可変サイズを固定化しない。

`source::audio` の責務は、届いた時系列の PCM を `f32` に揃えて渡すことに限定する。
チャンネル数・サンプルレートの変換はミキサー(`mixer::dsp` の `downmix_to_stereo` / `resample_stereo`)で行う。
エンコーダが固定フレームサイズを要求する場合(配信の AAC は 1024 サンプル)も、その直前で組み直す(`output::rtmp`)。

## ミキサーへの受け渡し

1. UI のソース一覧からミキサーへドラッグすると、`Mixer::add` でチャンネルが追加される
2. エンジンスレッドが `Live::sync(&catalog, mixer.sources())` で、ミキサーに入っているソースだけを開く
3. `Mixer::process` が各チャンネルの `Chunk` を取り出し、ステレオ 48kHz に揃えてフェーダーを掛けて合成する
4. 合成結果はモニター出力の `Bus` に積み、`Fanout` で配信・録画の各出力にも配る(出力ごとに自分用の `Bus` を持つ)。マスターのフェーダー・ミュートはどれにも効く
5. ミキサーから外すと `Live::sync` で `Feed` が drop され、ストリームや受信スレッドが止まる
