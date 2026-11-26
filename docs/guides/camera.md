# Aproximar a câmera de um splat

## Pela janela

1. Selecione o clipe de splat na timeline e abra **Câmera** no inspetor.
2. Clique em **Aproximar e pré-visualizar**. O editor avança suavemente 20% da distância até o alvo nos últimos 1,25 s do clipe e toca esse trecho no monitor, sem áudio.
3. Salve para manter o movimento. Use **Desfazer** ou o botão de remover ao lado da aproximação para voltar à câmera base.

O avanço usa o alvo e a escala da câmera do clipe. Funciona com splats comuns e com capturas SHARP, preservando o FOV, as chaves e a órbita existentes.

## Pela CLI

`amount` é a fração de avanço, acima de 0 e até 0,8. `frames` é o número de quadros finais do clipe, incluindo a pose inicial. Use pelo menos dois. Os dois parâmetros podem ser omitidos. O comando retorna `range: [início, fim)` com o trecho afetado.

```sh
./scripts/headless <<'JSONL'
{"op":"project.open","path":"/caminho/aventura.tessel.json"}
{"op":"clip.camera.push_in","clip":"take-final","amount":0.24,"frames":18}
{"op":"project.save","path":"/caminho/aventura.tessel.json"}
JSONL
```

Com a janela aberta, `./scripts/ui invoke camera.push_in` aplica o padrão e toca a prévia. Para remover pela CLI, envie `{"op":"clip.camera.push_in.clear","clip":"take-final"}`. O projeto com a aproximação salva exige um editor que leia o esquema 3.

[Prévia e exportação do trecho](preview.md) · [Contrato da câmera](../features/camera-push-in.md)
