# Rever só um trecho

Na janela, **Câmera → Aproximar e pré-visualizar** toca a parte alterada no monitor e para no último quadro do trecho. Use o controle de reprodução para interromper.

Com o projeto salvo e o ambiente preparado, use
`./scripts/tessel export aventura.tessel.json previa.mp4 --clip take-final`.
O comando acompanha o progresso no terminal. [Guia de exportação](export.md)

Para rodar o mesmo fluxo por JSONL, use o renderer.

```sh
./scripts/tessel render <<'JSONL'
{"op":"project.open","path":"/caminho/aventura.tessel.json"}
{"op":"export.start","camera_preview":"take-final","destination":"/tmp/previa-take-final.mp4"}
JSONL
```

`camera_preview` recebe o ID de um clipe com aproximação e escolhe o intervalo automaticamente. O preset padrão exporta sem áudio e com até 640 px no lado maior. Um preset explícito usa as configurações que você escolher.

Para rever qualquer outro corte, use `range` e um preset no lugar de `camera_preview`. O início é inclusivo e o fim é exclusivo. Este pedido exporta 30 quadros.

```json
{"op":"export.start","preset":"fullhd","range":[120,150],"destination":"/tmp/previa-corte.mp4"}
```

Quando encurtar uma aproximação existente, passe o `range` da resposta de `clip.camera.push_in` para conferir também os quadros onde o movimento anterior foi retirado. [Opções de exportação](../features/video-export.md)
