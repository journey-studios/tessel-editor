# Guias de uso do Tessel Editor

- **splat** · Gere um splat a partir de uma imagem e reutilize-o na timeline. [Gerar splats](splat.md)
- **camera** · Aproxime a câmera de um splat e ajuste a intensidade e a duração. [Aproximar a câmera](camera.md)
- **preview** · Revise no monitor ou exporte só os quadros que precisa conferir. [Prévia de um trecho](preview.md)

No terminal, use `./scripts/ui help camera`, `./scripts/media help splat` ou `./scripts/render help preview`. A ajuda abre sem uma janela do editor.

Dentro de uma sessão JSONL, envie `{"op":"help"}` para listar os tópicos ou `{"op":"help","topic":"camera"}` para ler um guia. O campo `markdown` traz os passos e exemplos.
