# Guias de uso do Tessel Editor

- **splat** · Gere um splat a partir de uma imagem e reutilize-o na timeline. [Gerar splats](splat.md)
- **camera** · Aproxime a câmera de um splat e ajuste a intensidade e a duração. [Aproximar a câmera](camera.md)
- **preview** · Revise no monitor ou exporte só os quadros que precisa conferir. [Prévia de um trecho](preview.md)
- **export** · Exporte um vídeo, intervalo ou prévia de câmera com progresso. [Exportar vídeo](export.md)
- **setup** · Prepare o runtime e confira as ferramentas de render. [Preparar o ambiente](setup.md)
- **package** · Arquive o projeto editável e suas mídias. [Empacotar o projeto](package.md)
- **restore** · Verifique o pacote e reabra o projeto em outra pasta. [Restaurar o projeto](restore.md)

Use `./scripts/tessel help` para listar os comandos e `./scripts/tessel help <tópico>` para ler um guia. A ajuda abre sem uma janela do editor.

Dentro de uma sessão JSONL, envie `{"op":"help"}` para listar os tópicos ou `{"op":"help","topic":"camera"}` para ler um guia. O campo `markdown` traz os passos e exemplos.
