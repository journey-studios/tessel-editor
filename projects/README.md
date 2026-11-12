# Projetos locais

`projects/<nome>/` guarda projetos do editor (`<nome>.tessel.json` mais a mídia) que **não** devem entrar no
repositório: conteúdo autoral ou privado, mídia pesada e saídas de render.

Este repositório é **público** (MIT) e já passou por saneamento de histórico por causa de material sem licença
verificada. Por isso:

- Tudo dentro de `projects/` é ignorado pelo Git, exceto este arquivo (regra no `.gitignore`). Não use
  `git add -f` e não copie esse conteúdo para `docs/`, `crates/` nem para mensagens de commit ou de PR.
- Um projeto aqui continua fora do escopo do repositório: nada daqui é fixture, exemplo nem dado de teste. Fixtures
  públicas vivem em `crates/*/tests/` e em `scripts/` e são sintéticas.
- Worktrees não compartilham `projects/`: ele existe só na worktree onde foi criado. Trocar de branch na mesma
  worktree não o altera (a pasta fica onde está), então um `project.save` pode sobrescrever o documento de mesmo
  nome criado sob outra branch.
- Para mover um projeto, reúna dentro da pasta a mídia que estiver fora dela, salve o documento de novo (só os
  caminhos dentro da pasta viram relativos) e copie a pasta inteira.

## Estrutura sugerida

```text
projects/<nome>/
  <nome>.tessel.json   documento do projeto (F022); caminhos de mídia relativos a esta pasta
  media/               mídia gerenciada pelo documento (imagens, áudio, vídeo)
  README.md            procedência, o que ficou de fora e como restaurar a mídia
```

Mídia ausente não impede abrir o projeto: o documento abre sem a mídia (conferido na CLI headless) e, segundo
[F022](../docs/features/project-document.md), a janela a marca como `missing`. A religação (`media.relink`,
[F019](../docs/features/media-io.md)) confere tamanho, SHA-256, tipo e duração: nome igual não basta.

## Criar um projeto

Pela CLI headless, que usa as mesmas regras da janela ([F015](../docs/features/project-creation.md)):

```sh
printf '%s\n' \
 '{"op":"project.create","name":"Exemplo","width":1280,"height":720,"seconds":10,"rate":24}' \
 '{"op":"project.save","path":"projects/exemplo/exemplo.tessel.json"}' | ./scripts/headless
```

O `project.save` cria a pasta do documento se ela não existir. Depois de adicionar assets, trilhas e clipes,
salve de novo com `project.save`; só os caminhos que estiverem dentro da pasta do documento viram caminhos
relativos.

## Montar por script

Um projeto grande sai melhor de um script em `projects/<nome>/tools/` que escreve operações JSON Lines e as
passa a `./scripts/headless` (vocabulário em [F015](../docs/features/project-creation.md)): `project.create`,
`asset.add`, `track.add`, `clip.add` e, no fim, `project.save`. Duas regras, conferidas com a CLI:

- `track.add` de vídeo põe a trilha **no topo**: a primeira criada fica embaixo, e as de áudio vêm depois das de
  vídeo. O `plan` compõe de baixo para cima, então crie primeiro a camada de fundo.
- Passe caminhos de mídia absolutos: o `project.save` grava como relativos os que estiverem dentro da pasta do
  documento. Confira que o JSON salvo não contém `/Users/` nem `/tmp/`.

Só descreva a edição no script; ele não renderiza nada.
