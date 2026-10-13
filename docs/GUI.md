# Interface gráfica (`k617rgb-gui`)

![Efeitos](gui-efeitos.png)
![Por tecla](gui-por-tecla.png)

## O que faz

- **Efeitos**: escolhe um dos 20 efeitos do teclado, a cor (ou "Aleatória"), a velocidade (1-4) e o brilho (0-4).
- **Por tecla**: pinta cada tecla com uma cor fixa. Clique ou arraste para pintar; Shift+clique copia a
  cor de uma tecla. São 5 grupos, todos salvos no próprio teclado (modo 21).
- Nada é enviado até você clicar em **Aplicar**. Cada escrita grava na memória do teclado, então a GUI
  não manda nada a cada clique.
- O estado da tela (cores pintadas, efeito escolhido) fica salvo em `~/.local/share/k617rgb/app.ron`,
  porque o teclado não tem comando para ler de volta as cores por tecla.

Limitação do hardware: não existe efeito animado numa tecla só (ex.: só o W "respirando"). Por tecla,
a cor é sempre fixa.

## Compilar

```bash
cargo build --release --features gui
./target/release/k617rgb-gui
```

O CLI (`cargo build --release`, sem a feature) continua sem nenhuma dependência.

## Por que Rust + eframe/egui

| Opção | Avaliação |
|---|---|
| **eframe/egui** (escolhida) | Mesma linguagem do projeto: a GUI chama direto as funções de `src/lib.rs` (`mode_packets`, `custom_packets`, `read_cfg`). Binário único (~11 MB, sem runtime), abre na hora e desenha na GPU (OpenGL). Funciona nativo em Wayland/Hyprland e X11. O teclado é desenhado com retângulos a partir das coordenadas do próprio INI do fabricante. O color picker já vem pronto. |
| iced | Também é Rust e bonito, mas a arquitetura Elm (Message/update/view) dá mais código para uma tela só. |
| GTK4 + libadwaita | Visual GNOME ótimo, mas fora do GNOME (Hyprland, KDE) o tema não integra bem. Traz uma dependência de sistema pesada, e um canvas de teclado clicável dá mais trabalho. |
| Slint | Bom e leve, mas adiciona uma DSL própria (`.slint`) e um build script. Ganho pequeno aqui. |
| Tauri / Electron | Interface web: 100+ MB de RAM e um webview inteiro para uma tela. Vai contra o objetivo de ser leve. |

## Como está organizado

- `src/lib.rs`: protocolo (pacotes HID, hidraw, tabela de teclas com LED e posição). É usado pelo CLI e pela GUI.
- `src/main.rs`: CLI.
- `src/bin/k617rgb-gui.rs`: GUI. A comunicação com o teclado roda numa thread separada (`std::sync::mpsc`),
  para a janela não travar enquanto os pacotes são enviados.
