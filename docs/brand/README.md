# Marca do Glint

Guia curto para usar o nome, o símbolo e as cores do Glint sem inventar variações.

## Símbolo e wordmark

| Arquivo | Uso |
|---|---|
| `simbolo.svg` (= `icone.svg`) | Símbolo: lente de vidro com reflexo de 4 pontas num squircle grafite |
| `icone-pequeno.svg` | O mesmo símbolo simplificado para 16–24 px (sem o anel fino, realce mais grosso) |
| `wordmark-escuro.svg` | Símbolo + `glint` para fundo escuro (texto `#f5f5f7`) |
| `wordmark-claro.svg` | Símbolo + `glint` para fundo claro (texto `#111114`) |
| `../../assets/brand/glint.ico`, `glint-512.png`, `glint-1024.png` | Gerados por `cargo run -p make-icons` (não editar à mão) |

- O nome se escreve **`glint`, minúsculo**, em Inter SemiBold com tracking −0,045 em. Em frase
  corrida, "Glint" com maiúscula (como qualquer nome próprio).
- No wordmark o símbolo tem a altura do corpo do texto e fica a 30% dessa altura do nome.
- **Respiro:** em volta do símbolo ou do wordmark, deixe livre pelo menos ¼ da altura do símbolo.
- **Tamanho mínimo:** símbolo com 16 px (abaixo de 24 px use `icone-pequeno.svg`); wordmark com 20 px
  de altura.

## O que não fazer

- Colorir o símbolo ou o nome (nada de azul, vermelho ou degradê de marca).
- Usar o vermelho como cor da marca: vermelho é **só sinal** (gravando, erro, apagar).
- Esticar, girar, contornar ou pôr sombra colorida no símbolo.
- Trocar a fonte do wordmark ou escrever `GLINT`/`Glint` no lugar do `glint` desenhado.
- Pôr o símbolo sobre fundo que esconda a borda do squircle (use o fundo neutro do tema).

## Cores (tokens)

Neutro: branco, preto e cinza. Azul leve só para foco, progresso e seleção.

| Token | Escuro | Claro |
|---|---|---|
| Janela (sobre o acrílico) | `rgba(20,21,25,0.58)` | `rgba(246,247,250,0.56)` |
| Painel | gradiente branco `0.10 → 0.045 → 0.035` | branco `0.85 → 0.55 → 0.48` |
| Borda de painel | `rgba(255,255,255,0.11)` | `rgba(255,255,255,0.90)` |
| Texto / secundário | `#f5f5f7` / `rgba(235,235,245,0.64)` | `#111114` / `rgba(50,50,60,0.72)` |
| Botão principal | branco `#fff → #e4e5e9`, texto `#111114` | grafite `#2a2b30 → #111114`, texto branco |
| Azul (foco/progresso) | `#7fb0ff` | `#2f6fe0` |
| Vermelho (sinal) | `#ff453a` | `#e5322b` |

Raios: janela 16, painéis 20–22, cards 16, campos 12, pílulas e botões redondos. No app, alturas de
controle só da escala `theme::size::Size` (24/32/40/46).

## Tipografia

- **Inter** (Regular, Medium, SemiBold) para tudo; **Geist Mono** (Medium) para números: tempos,
  tamanhos, contagens. Ambas OFL (`crates/app-ui/assets/fonts/OFL.txt`).
- Títulos com tracking levemente negativo (−0,02 a −0,03 em); texto corrido sem tracking.

## Tom dos textos

- Direto e em PT-BR do dia a dia: "Clipe salvo", "Publicar e copiar link", "Gravador fechado".
- Sem gíria de marketing, sem exclamação, sem "incrível"/"poderoso".
- Erro diz o que houve e o que fazer: "Não deu para publicar: sem internet. Tentar de novo".
- "clipe" é palavra comum (minúscula); "Glint" é o nome do app.
