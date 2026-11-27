# F077 — Fluxos de projeto no CLI

`./scripts/tessel` reúne exportação com progresso, configuração do runtime,
empacotamento e restauração. Os comandos usam Python 3 para encaminhar
argumentos e falar com os processos nativos. As regras de render, integridade e
publicação ficam nos mesmos backends usados pela janela e por JSONL.

## Comandos

```sh
./scripts/tessel setup
./scripts/tessel export aventura.tessel.json previa.mp4 --clip take-final
./scripts/tessel package aventura.tessel.json aventura.tesselpack
./scripts/tessel restore aventura.tesselpack aventura-restaurada
```

Use `tessel help export`, `help setup`, `help package` ou `help restore`
para ler os guias. `tessel <comando> --help` mostra a sintaxe e opções.
Caminhos relativos são resolvidos na pasta do chamador. O resultado em JSON
vai para stdout e o progresso ou erro para stderr. Um erro de uso sai com 2,
uma falha do backend com 1 e um job cancelado por Ctrl+C com 130.

## Contrato dos backends

- **Exportação.** `project.open` e `export.start` no renderer nativo. `--range`
  seleciona `[início,fim)` em quadros. `--clip` seleciona a aproximação de câmera
  salva (`camera_preview`). Os dois seletores são exclusivos. O padrão é
  `fullhd` para vídeo ou intervalo e o preset de prévia da câmera para clipe.
  `--preset`, `--sequence`, `--gl`, `--renderer` e `--replace` são opcionais.
  [F029](video-export.md) · [F075](camera-push-in.md)
- **Setup.** Executa o setup do runtime do repositório e consulta `tessel render
  --check`. Esta rota usa `Tools::discover`, executa `ffprobe -version` e obtém
  as capacidades do renderer via handshake real do Chrome com WebGL. O
  relatório apresenta o bundle escolhido por `TESSEL_RUNTIME_DIR` quando
  configurado. `setup --check` só roda as verificações. [Guia de setup](../guides/setup.md)
- **Pacote.** `project.open` e `project.package` no host de mídia. O padrão
  é arquivo `.tesselpack` completo, com opção de pasta e seleção de sequências.
  O destino existente é protegido, com troca explícita via `--replace`.
  [F026](package.md)
- **Restauração.** `project.restore` verifica e extrai o pacote para uma pasta
  nova. O relatório retorna `document` para abrir a cópia editável. O comando
  usa `open:false` porque a sessão headless encerra após concluir. [Guia de restauração](../guides/restore.md)

Os três comandos com jobs acompanham `job.get` e usam `job.cancel` ao receber
Ctrl+C. Um job que já publicou a saída mantém seu resultado. O script
`export-video.py` continua aceitando a chamada anterior com renderer pré-compilado.
A janela e as chamadas Rust usam os contratos de F026, F029 e F075.

## Evidências locais

- Exportes reais de 8 quadros inteiros, 2 por intervalo e 4 pela aproximação,
  todos aprovados pelo FFprobe. Ctrl+C durante render encerrou com 130,
  limpou o staging e preservou o arquivo de destino anterior. Se o cancelamento
  chega após a publicação, o teste do adaptador confirma que o resultado continua disponível.
- Setup completo e `--check` no Mac com FFmpeg, FFprobe, Node e Chrome reais.
  GPU e SwiftShader foram verificados. Um override de FFprobe inexistente
  foi recusado com diagnóstico. A instalação com pnpm 12 usa a permissão
  de build da versão de esbuild fixada no lockfile.
- Pacote em ZIP e em pasta, ambos restaurados com as mídias originais indisponíveis.
  O plano do projeto restaurado contém o splat. Destinos existentes foram recusados.
- A ajuda de cada tópico bate com o guia em disco. Os comandos de arquivo
  foram executados fora do repositório, com caminhos contendo espaços.
  A entrada anterior `export-video.py` também exportou um intervalo.
