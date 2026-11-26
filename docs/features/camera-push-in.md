# F075 — Aproximação de câmera e prévia de trecho

[Guia de uso pela janela e pela CLI](../guides/camera.md) · `./scripts/tessel help camera`

## Comportamento

**Aproximar e pré-visualizar**, no inspetor de Câmera, aplica uma aproximação aos últimos 1,25 s do clipe selecionado. A duração se adapta a clipes curtos. A câmera percorre 20% da distância até o alvo, com aceleração e desaceleração cúbicas. O movimento parte da câmera avaliada em cada quadro, conserva o alvo, o FOV, as chaves e a órbita, e funciona com splats com ou sem metadados de captura. O enquadramento do assunto vem da câmera do clipe.

Na janela, a prévia toca no monitor, em silêncio, do quadro inicial ao último quadro do trecho. Ela aguarda o primeiro quadro quando o monitor nativo está visível e precisa carregar a cena. O botão de remover apaga a aproximação. Aplicar novamente atualiza a mesma aproximação. Cada alteração é uma entrada no histórico.

## Contrato da biblioteca

`Command::SetCameraPushIn { clip, options: Some(PushInOptions { amount, frames }) }` aplica ou substitui o movimento. `options: None` remove. `amount` é uma fração em `(0, 0.8]`. `frames` é o número inteiro de quadros finais do clipe, incluindo a pose original, com mínimo de dois. Os dois campos são opcionais.

A operação devolve `changed`, `value` e `range: [início, fim)` em quadros da sequência. Ao substituir, o trecho inclui a extensão da aproximação anterior. `EditorState::camera_push_in_preview_range(clip)` retorna o trecho visível da aproximação atual, calculado para a posição, aparagem e velocidade do clipe. Uma edição inválida deixa o documento intacto. `invalid_time` indica que o trecho não cabe. `not_applicable` indica um clipe sem câmera ou, ao pedir só a prévia, sem movimento visível.

O efeito é salvo em `camera_push_in` com os limites em segundos racionais da fonte. O avanço fica constante depois da última pose quando o clipe é estendido. Documentos com essa propriedade usam esquema 3. Projetos sem a propriedade continuam nos esquemas anteriores. Capturar uma chave grava a câmera base para que o efeito seja aplicado uma única vez. O core dispensa GPUI e o renderer.

## CLI headless

Envie estas linhas à mesma sessão de `./scripts/tessel render`, depois de `project.open`:

```jsonl
{"op":"clip.camera.push_in","clip":"s6-torre","amount":0.24,"frames":18}
{"op":"export.start","camera_preview":"s6-torre","destination":"/tmp/previa-torre.mp4"}
```

`camera_preview` resolve o intervalo automaticamente e usa o export de intervalo existente. O preset padrão mantém a proporção da sequência, com lado maior de até 640 px, e exporta sem áudio. Um `preset` explícito usa as suas preferências. Para exportar também o começo que mudou ao encurtar uma aproximação anterior, use o `range` retornado pela edição no lugar de `camera_preview`, com um preset de exportação. A edição e a remoção (`clip.camera.push_in.clear`) também funcionam em `./scripts/tessel headless`. O MP4 requer o renderer e o FFmpeg.

## Janela e automação

Selecione um clipe de cena, abra **Câmera** e clique em **Aproximar e pré-visualizar**. O equivalente é `./scripts/tessel ui invoke camera.push_in`. O botão tem o nó `camera.push_in.clear` para remoção. `playback.status` expõe o intervalo da prévia e o estado do monitor. A prévia pode ser interrompida pelo controle de reprodução.

## Evidências locais

- Core: três testes em `crates/core/tests/camera_push_in.rs` conferem a proporção em duas escalas, órbita, chaves, velocidade, reabertura, recusa atômica, substituição e undo/redo.
- CLI sem janela: exportação de 18 quadros do PLY real com chaves de câmera em `s6-torre`. O FFprobe confirmou 640 × 360 e 0,6 s. O plano antes e depois mostrou a aproximação configurada de 24% e alvo e FOV preservados. A câmera do quadro 948 ficou idêntica após dividir o clipe no quadro 934, salvar e reabrir.
- CLI sem janela: SPZ comum de 4.000 gaussianas com órbita de 32°/s. O plano confirmou avanço de 35% em direção ao alvo e o export verificou os 12 quadros de `[22, 34)`.
- Janela nativa no Mac, via socket semântico: o mesmo take preparou, tocou e parou no intervalo `[922, 960)`, com playhead no quadro 959. A janela usou sua WebView real. O teste GPUI também confere o intervalo, o modo mudo, a parada e a remoção.
- Suíte do workspace no Mac com Chrome e FFprobe locais: 3.265 passaram, 17 ignorados e zero falhas. Build e formatação passaram. Clippy estrito encontrou `large_enum_variant` em `Op`, reproduzido no commit base. As demais regras passaram com essa exceção na chamada do Clippy.
