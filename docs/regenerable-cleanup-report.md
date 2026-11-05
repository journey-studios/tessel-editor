# Limpeza de dependências e builds regeneráveis

Executada em 2026-11-04T22:24:45 após autorização explícita do usuário.
Escopo: `~/www/journeystudios` e `/private/tmp`, somente diretórios `node_modules` e targets
Cargo reconhecidos, com todos os arquivos/diretórios modificados há mais de 24 horas.
Symlinks não foram seguidos/removidos como alvos; Cargo exigiu manifesto ancestral e marcadores de build.

## Resultado

- Removidos: 1011 diretórios node_modules e 5 targets Cargo.
- 74 candidatos elegíveis preservados por uso do projeto.
- Na triagem: 76 diretórios recentes preservados;
  outros skips e motivos no plano temporário. Um target não confirmado como Cargo ficou preservado.
- Todas as remoções foram verificadas: caminhos já não existem.
- `df` no final: **5,7 GiB livres**. Antes da limpeza oscilava entre 2,4 e 2,6 GiB.
  Ganho observado aproximado: **3 GiB**, com outros agentes ainda ativos.
- Soma dos blocos dos diretórios removidos: 19,36 GiB; **isso não é espaço físico recuperado**,
  pois dependências podem compartilhar hardlinks/clones. Não somar target symlink ao cache mbx.

## Proteções

Checados processos, working directories e arquivos abertos por lsof; projetos ativos e arquivos
rastreados pelo Git foram excluídos da limpeza. Sources e manifests ficaram preservados.
O node_modules do Tessel principal foi preservado por estar em uso. Não foram removidos
worktrees, arquivos de projetos, mídias, imagens de disco, perfis Chrome nem caches globais.
O cache mbx e target do tessel-editor estão fora desta limpeza.

Targets removidos:

- `/Users/wallyssonnunes/www/journeystudios/paperclip-plugin-host/packages/paperclip-runner/runner/target`
- `/Users/wallyssonnunes/www/journeystudios/paperclip-project-hierarchy/packages/paperclip-runner/runner/target`
- `/private/tmp/vgp16/third_party/sprite-normals/target`
- `/Users/wallyssonnunes/www/journeystudios/simulation-engine-laya-vision/target`
- `/Users/wallyssonnunes/www/journeystudios/moodio/apps/studio/src-tauri/target`

[Auditoria por caminho](maintenance/regenerable-cleanup-audit.json). Para voltar a desenvolver
nos projetos limpos, reinstalar dependências via pnpm e reconstruir os crates via Cargo.
