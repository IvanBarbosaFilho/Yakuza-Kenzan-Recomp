# Bugs do build Windows: o que passar para o Claude no Linux

Cole a mensagem abaixo no Claude do Linux, dentro da pasta do repositório. Leve também o arquivo
`engine/libs/audio/cellAudio.c` alterado, ou aplique à mão o trecho da seção "A correção".

---

## Mensagem

No Windows (Windows 10, AMD RX 7600, build Kenzan-Windows feito com `packaging/windows/build_windows.sh`)
tenho dois problemas que não acontecem no Linux:

1. **Som estourado / estalando.**
2. **Às vezes a tela fica piscando quando o jogo abre** (`display_mode=fullscreen`).

Diagnóstico já feito no Windows, a partir do log `work/logs/last_run_out.txt`:

- Áudio: `output device: 44100 Hz, 2 channel(s), 32-bit`, `converted by Windows, event-driven, 2880-frame buffer`,
  e o jogo abre `PortOpen(nChannel=8, nBlock=8)`.
- A causa provável é o loop de mixagem do Windows em `engine/libs/audio/cellAudio.c` (`audio_mix_thread_func`,
  ramo `_WIN32`, backend WASAPI). Ele mistura sempre que a placa tem espaço, então enche o buffer de 60 ms
  inteiro de uma vez: 11 blocos de 256 amostras seguidos no começo, depois ~2 a cada período de 10 ms. O anel do
  jogo tem só 8 blocos, então o mixer dá a volta no escritor do jogo e toca blocos zerados ou escritos pela
  metade. No Linux o backend é o SDL, e o loop POSIX mantém no máximo ~4 blocos na fila e nunca passa à frente
  do relógio. Por isso lá não acontece.
- A correção já está aplicada no `cellAudio.c` que eu trouxe (veja abaixo). Ela **não foi compilada nem testada**.
  Por favor:
  1. Revise a mudança e confira se compila no build Windows.
  2. Compile só o engine: `bash packaging/windows/build_windows.sh engine`.
  3. Me diga qual arquivo copiar para o Windows: `work/win/build-engine/KenzanRecomp.exe`, que vai por cima de
     `engine/KenzanRecomp.exe` na pasta do jogo.
- Tela piscando: o engine cria a janela com `SDL_WINDOW_FULLSCREEN_DESKTOP` (`vk_window_open_locked` em
  `engine/libs/video/rsx_vulkan_backend.c`). No Windows, uma janela que cobre o monitor inteiro pode ser
  promovida a independent flip e sair do compositor (DWM), o que costuma piscar no AMD. O engine já tem o modo
  `PS3RECOMP_VK_CAPTURE_FRIENDLY=1` (janela sem borda, 1 px mais baixa). Vou testar esse modo no Windows. Se
  resolver, quero que ele vire o padrão do fullscreen no Windows (no engine ou no `launcher/src/config.rs`,
  `env()`), sem mudar nada no Linux.
- Outras informações: o `kenzan.ini` pede `present=mailbox`, mas o driver AMD no Windows não oferece esse modo
  e o jogo cai para `fifo`. Isso é esperado, não é bug.

---

## A correção (já aplicada no cellAudio.c que eu trouxe)

Em `audio_mix_thread_func` (ramo `#ifdef _WIN32`), o começo do loop foi trocado por este trecho. Com placa de
som, o loop mistura um bloco a cada 5,33 ms pelo QueryPerformanceCounter e mantém a placa entre 4 blocos e
6 blocos + 480 frames na fila. Quando a fila baixa, completa na hora e reancora o relógio. Quando atrasa mais
de 2 blocos, reancora em vez de disparar uma rajada. `PS3_AUDIO_PACE=device` volta ao comportamento antigo.

```c
    const char* pace_env = getenv("PS3_AUDIO_PACE");
    const int pace_device = pace_env && !strcmp(pace_env, "device");
    LARGE_INTEGER qf, q0; QueryPerformanceFrequency(&qf); QueryPerformanceCounter(&q0);
    const long long blk_ticks = (long long)CELL_AUDIO_BLOCK_SAMPLES * qf.QuadPart / CELL_AUDIO_SAMPLE_RATE;
    unsigned long long blocks = 0;
    while (s_mix_thread_running) {
        const int room = audio_backend_room();
        if (room >= 0 && room < CELL_AUDIO_BLOCK_SAMPLES) {
            if (pace_device) audio_backend_wait(); else Sleep(1);
            continue;
        }
#if AUDIO_BACKEND_WASAPI
        if (room >= 0 && !pace_device) {
            const int queued = (int)s_wasapi_buf_frames - room;
            const int lo = CELL_AUDIO_BLOCK_SAMPLES * 4;
            int hi = CELL_AUDIO_BLOCK_SAMPLES * 6 + 480;
            if (hi > (int)s_wasapi_buf_frames - CELL_AUDIO_BLOCK_SAMPLES)
                hi = (int)s_wasapi_buf_frames - CELL_AUDIO_BLOCK_SAMPLES;
            LARGE_INTEGER now; QueryPerformanceCounter(&now);
            const long long due = q0.QuadPart + (long long)blocks * blk_ticks;
            if (queued >= lo) {
                if (queued >= hi || now.QuadPart < due) { Sleep(1); continue; }
                if (now.QuadPart - due > 2 * blk_ticks)
                    q0.QuadPart = now.QuadPart - (long long)blocks * blk_ticks;
            } else {
                q0.QuadPart = now.QuadPart - (long long)blocks * blk_ticks;
            }
        }
#endif
        if (room < 0) {
            /* ... o ramo sem placa continua como estava ... */
```

O que era antes:

```c
    LARGE_INTEGER qf, q0; QueryPerformanceFrequency(&qf); QueryPerformanceCounter(&q0);
    unsigned long long blocks = 0;
    while (s_mix_thread_running) {
        const int room = audio_backend_room();
        if (room >= 0 && room < CELL_AUDIO_BLOCK_SAMPLES) { audio_backend_wait(); continue; }
        if (room < 0) {
```

Se o som continuar ruim depois disso, rode no Windows com `AUDIO_RATE=1` (blocos por segundo e frames
descartados) e `AUDIO_WAV=<arquivo>` (grava a mixagem exata antes da placa). Depois mande o
`work/logs/last_run_err.txt` e o arquivo gravado.
