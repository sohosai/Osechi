# ミュートAPI

donguri(音響卓側システム)など外部システムから、Osechi のマスターミュートを操作するための HTTP API。
実装は `src/api/`。

## 起動

- アプリ起動時に専用スレッドで自動的に起動する
- listen アドレスは `0.0.0.0:7878`。ポートは環境変数 `OSECHI_API_PORT` で変更できる(不正な値なら 7878)
- ポートの bind に失敗してもログに `error` を出すだけで、アプリ本体は起動する

```sh
OSECHI_API_PORT=8080 cargo run
```

## エンドポイント

### `GET /mute`

マスターミュートの現在の状態を返す。状態は変更しない。

```sh
curl http://localhost:7878/mute
```

```json
{"is_muted": false}
```

### `POST /mute`

マスターミュートの状態を変更し、変更後の状態を返す。

```sh
curl -X POST http://localhost:7878/mute \
  -H 'Content-Type: application/json' \
  -d '{"is_muted": true}'
```

```json
{"is_muted": true}
```

| ステータス | 条件 |
|---|---|
| `200 OK` | 更新した |
| `415 Unsupported Media Type` | `Content-Type: application/json` が無い |
| `422 Unprocessable Entity` | `is_muted` が無い・真偽値でない |

## OpenAPI

- スキーマ: `http://<host>:<port>/api-docs/openapi.json`
- Swagger UI: `http://<host>:<port>/swagger-ui`

## CORS

すべてのオリジン・HTTPメソッド・ヘッダーを許可(`CorsLayer::permissive()`)しています。ブラウザ上のWebフロントエンドから直接リクエストを送ることができます。

## UI との同期

API とミキサーは `Arc<AtomicBool>` を共有し、エンジンスレッドが約 10ms ごとに `Mixer::process` の中で突き合わせる。

- 前回揃えた値から変わった側を採用する
- 同じ間隔の中で両方が変えていた場合は API 側を優先する
- UI でミュートを切り替えた結果も `GET /mute` に反映される

マスターミュートはモニター出力・配信・録画のすべてに効く。
