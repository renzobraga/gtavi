# Num PC de 16 GB com uma RTX 5060 Ti de 8 GB (Ryzen 5 7600X)

Guia para rodar este fork num PC menor que o de desenvolvimento: Ryzen 5 7600X
(6 núcleos, 12 threads, AVX-512), 16 GB de RAM, GeForce RTX 5060 Ti de 8 GB e
um monitor de taxa variável (G-SYNC / FreeSync, aqui um TCL de 300 Hz).

## O que esperar, sem rodeios

- **Nada disto foi testado nesse hardware.** O fork só foi medido num i9-12900K
  com 64 GB de RAM e uma RX 7900 XTX de 24 GB. Lá ele usa cerca de 30 GB de
  memória privada e 10,5 GB de VRAM. Num PC de 16 GB, rodar já é a primeira
  meta. Desempenho bom vem depois.
- O que foi feito: o launcher agora detecta um PC menor e escolhe sozinho o
  perfil e os ajustes que economizam memória. A camada também passou a reagir
  ao orçamento de memória da placa e do PC. Os grupos de threads foram
  dimensionados para 6 núcleos, e o processador usa AVX-512 onde o tem.
  Em placas NVIDIA, o que falha agora avisa por quê, em vez de falhar calado.
- Tudo é automático e respeita o que você definir à mão: qualquer `VICTORY_...`
  que você passar vale mais que a escolha do launcher.
- **Quantos fps, numa estimativa (não medida):** no PC de desenvolvimento, o
  melhor resultado é cerca de 22 fps numa rua da cidade. Aqui a placa tem mais
  ou menos metade da força, então espere **uns 11 a 14 fps** sem o `-Perf`. Com o
  `-Perf` (item 4), algo como **13 a 15 fps**, com menos quadros longos. Indo
  aos experimentos de resolução (item 4), talvez **16 a 19 fps**. Os 30 fps não
  estão ao alcance desta placa com a camada como ela está hoje.
- **Abaixo de 15 fps o tempo do jogo fica mais lento que o real.** O título
  limita cada passo da simulação a 1/15 s. Por isso, aqui, ficar abaixo de
  66 ms por quadro importa tanto quanto o fps.

## 1. Preparar o Windows (uma vez)

**Arquivo de paginação: o mais importante.** O título precisa de muito *commit*
(RAM + arquivo de paginação). Com 16 GB de RAM e o arquivo gerenciado pelo
Windows, a execução tende a acabar por falta de memória (erro 1455 no
`crashes.log`).

1. Win + R, digite `sysdm.cpl` e dê Enter.
2. Abra **Avançado → Desempenho → Configurações… → Avançado → Memória
   virtual → Alterar…**
3. Desmarque **Gerenciar automaticamente…** e escolha o SSD mais rápido (NVMe).
4. Selecione **Tamanho personalizado** com inicial e máximo de **32768 MB**
   (ou 49152 MB se houver espaço). Clique em **Definir** e reinicie.

O launcher confere isso ao iniciar. Se o limite de *commit* estiver abaixo de
RAM + 24 GB, ele imprime um `warning` dizendo quanto falta.

**Antes de jogar**, feche o navegador, o Discord, os launchers (Steam, Epic,
etc.) e os overlays. Cada GB livre conta.

**Driver e painel da NVIDIA**

- Instale um driver recente.
- Em **Painel de Controle NVIDIA → Gerenciar as configurações 3D → Tamanho do
  cache de shader**, escolha **10 GB** ou **Ilimitado**. As pipelines do título
  são muitas, e um cache pequeno faz o jogo engasgar de novo a cada execução.
- Em **Configurar G-SYNC**, ative o G-SYNC para **janela e tela cheia**. No menu
  do monitor (OSD), ligue o Adaptive-Sync/VRR e use o cabo DisplayPort.

**BIOS:** deixe o perfil **EXPO** da memória DDR5 ligado. A camada copia muita
memória, e o Zen 4 sem EXPO perde banda.

**Energia:** coloque o modo de energia do Windows em **Melhor desempenho**.

## 2. Compilar

Os pré-requisitos e o clone estão no [README](../README.md#how-to-run-it). Há
dois builds possíveis:

```powershell
# diag: o build de todas as medições, com os logs de diagnóstico (comece por este)
cargo build --locked --profile diag -p victory --features diagnostics

# release: LTO thin com uma unidade de código, sem os interruptores de depuração
cargo build --locked --release -p victory
```

O build release usa bastante RAM ao linkar. Feche tudo antes. Se ele acabar
por falta de memória, use `cargo build -j 4 ...`. Nenhuma medição deste fork foi
feita com ele. Só passe a usá-lo depois de uma sessão estável no diag. Ele não
tem DRED, então o `crashes.log` diz menos.

## 3. Antes da primeira execução

```powershell
target\diag\victory.exe shaders <os arquivos de shader do título>   # traduz todos, leva minutos
target\diag\victory.exe warm --used                               # pipelines das execuções anteriores
```

- `warm --used` só tem efeito a partir da segunda execução. Rode-o de novo
  depois de atualizar o driver ou este branch.
- Enquanto traduz, `victory shaders` avisa de cada programa de *compute* que
  uma placa de ondas de 32 (NVIDIA) recusa ou pode recusar, com o hash dele
  (linhas com `32-lane waves`). A linha final só dá as contagens. Só a primeira
  tradução de cada programa gera esse aviso: uma execução que acha a tradução
  já guardada não o repete. **Guarde a saída inteira da primeira execução**
  (`... *>&1 | Tee-Object shaders.log`), porque ela é a primeira coisa a
  mandar (veja o item 7).

## 4. Jogar

```powershell
scripts\play.ps1 -Exe "D:\caminho\da\sua\cópia\GTA6.exe" -Perf -Vrr *>&1 | Tee-Object -FilePath run.log
```

- `-Perf` é o **modo desempenho**. Ele liga os patches 2, 6 e 7 do renderer
  (README), que tiram trabalho da thread de envio, a que manda o trabalho do
  título para a placa:
  - **2:** acaba com uma varredura de cerca de 600.000 descritores por quadro.
    Ela custava 15–16 ms de uma thread, e aqui essas threads são só 3.
  - **6:** as travas de memória deixam de esperar umas pelas outras.
  - **7:** o diário de escritas é dividido em 64 partes.

  Nenhum dos três depende da marca da placa nem muda os shaders, então as
  traduções e as pipelines já feitas continuam valendo. O patch 5 fica de fora:
  ele precisa de um heap de views que a NVIDIA não tem (o launcher avisa se você
  o ligar). Se algo der errado, tire o `-Perf`. Com `-Safe`, ele é ignorado.
- `-Vrr` permite *tearing* na troca de quadros, para que o G-SYNC acompanhe a
  taxa de quadros também em janela.
- `| Tee-Object` guarda o log em `run.log` e mostra o mesmo texto na tela.
- Para usar o build release, acrescente `-Build release`.

Ao iniciar, confira estas linhas:

```
host           ... RAM, commit, NVIDIA GeForce RTX 5060 Ti (vendor 0x10de) ~7.8 GiB, 12 lógicos / 6 núcleos, AVX-512, build do Windows
console        auto: Xbox Series S, ...
host preset    VICTORY_RESIDENCY=1: ...
host preset    ... (uma linha por ajuste)
warning        ... (só se faltar commit ou se a resolução de render passar do limite da NVIDIA)
```

### O que o launcher escolhe sozinho neste PC

| O quê | Escolha | Por quê |
|---|---|---|
| Perfil do console (`--console auto`) | **Series S** | Com 24 GB ou menos de RAM, ou uma placa de 10 GB ou menos, o perfil Series X (13,5 GiB de memória de console para o título, saída 4K; o Series S dá 7,6 GiB) não cabe |
| Upscaler | `VICTORY_UPSCALER=6`: o upscaler do próprio título, 1280×720 → 1920×1080 | Os shaders de FidelityFX de console do Series S só rodaram em AMD até hoje. Só vale quando o perfil foi escolhido no automático |
| Memória do PC | `VICTORY_RESIDENCY=1`, `VICTORY_BURY_LAG=64`, `VICTORY_WINDOW_KB=1024`, `VICTORY_WINDOW_TAILS=1` | Orçamentos de memória ligados, janelas de memória menores, sem liberar objetos cedo demais |
| Memória da placa | `VICTORY_LEVELS_ON_DEMAND=1` | Níveis de mip das texturas ocupam VRAM só quando usados (2 a 3 GB a menos esperados) |
| Placa NVIDIA | `VICTORY_BOOK=0` | O "livro" de memória da placa só decodifica descritores AMD |
| Rastreamento de falhas | `VICTORY_D3D12_DRED=faults` | Só o endereço da falha, sem *breadcrumbs*, que custam memória |
| Prioridades | As threads ociosas do título sobem para "abaixo do normal" | Em 12 threads lógicas, as seis threads ociosas do título (o *pager* de streaming entre elas) esperavam segundos pela vez |

Dentro da camada, sem nenhuma variável:

- **Orçamento de VRAM.** A camada trabalha com o orçamento que o Windows dá à
  placa menos uma reserva de 384 MB ou 5 %, o que for maior (≈7,3 → ≈6,9 GB).
  O espaço para *render targets* cai de 3 GB para cerca de 3/10 do orçamento.
- **Memória compartilhada.** O uso da memória do PC que a placa compartilha é
  acompanhado: de 70 % em diante a camada guarda menos sobra, e a partir de
  85 % ela libera o que está parado, uma vez ao entrar nessa faixa.
- **Allocators.** Os allocators de listas de comandos que incharam são
  recriados.
- **Falta de memória.** Diante de um `E_OUTOFMEMORY`, a camada libera o que
  pode e tenta uma segunda vez antes de desistir.
- **Grupos de threads para 6 núcleos.** Tradução de listas 5, pipelines 4 + 2,
  registros do heap 3, e no máximo 4 descompressões de DirectStorage ao mesmo
  tempo. As threads de leitura continuam uma por thread lógica.
- **AVX-512.** As cópias e comparações de memória do título usam AVX-512.

### Para mudar alguma escolha

| Quero… | Use |
|---|---|
| O perfil Series X (saída 4K, muito pesado para 8 GB) | `-Console series-x`, e de preferência `-More @{VICTORY_OUTPUT='1920x1080'}` |
| Series S com o FidelityFX de console | `-Console series-s` (sem o `VICTORY_UPSCALER=6`) ou `-More @{VICTORY_UPSCALER='7'}` |
| Nenhuma das escolhas do launcher e do `play.ps1` (a camada continua se adaptando, veja abaixo) | `-More @{VICTORY_HOST_PRESET='0'}` |
| Passar de 30 fps (só faz diferença acima de 30) | `-More @{VICTORY_FRAME_INTERVAL_MS='16.667'}` |
| O modo seguro | `-Safe` |

**Não mude a prioridade do processo no Gerenciador de Tarefas.** A camada já
roda acima do normal e desliga o *power throttling* do Windows por conta própria.

### Mais fps: experimentos, um de cada vez

Com o `-Perf`, quem limita o quadro aqui passa a ser a placa (estimativa: 65 a
75 ms por quadro). O que mais ajuda é dar menos trabalho a ela. Nada abaixo foi
rodado no perfil Series S, então tente um item de cada vez, sempre na mesma
rua, e volte atrás se a imagem quebrar.

| # | Experimento | O que deve mudar | Risco |
|---|---|---|---|
| 1 | `-Output 1664x936` | Imagem de saída menor, render interno de ~1109×624: estimativa de 2,5 a 4 ms a menos de placa e menos VRAM | Imagem e HUD mais suaves |
| 2 | `-Output 1280x720` | Render interno de 853×480: estimativa de 5 a 8 ms a menos | Imagem bem mais suave |
| 3 | `-More @{VICTORY_RENDER_SCALE='0.6'}` | Render interno de 1152×648 em vez de 1280×720 (com saída 1080p): estimativa de 1,5 a 2,5 ms a menos | O upscaler do título só foi rodado em 2/3 e em 1,0 |
| 4 | `-More @{VICTORY_PORT_CONSTANTS='1'}` (patch 1) | Libera cerca de 174 MB de RAM e até ~348 MB de VRAM | A placa passa a ler o heap de constantes pelo PCIe x8: pode ganhar ou perder até ~6 ms. Provavelmente pede um Windows 11 recente (24H2) |
| 5 | `-More @{VICTORY_CARD_WINDOWS_MB='384'; VICTORY_TEXTURE_ROOM_PERCENT='80'}` | Cópias da memória do título passam a ficar na VRAM, sem cruzar o PCIe a cada desenho | Menos VRAM para texturas, que podem aparecer mais tarde |
| 6 | `-More @{VICTORY_D3D12_BACKGROUND='0'}` | O driver para de recompilar shaders em segundo plano, o que pesa em 6 núcleos | Nunca medido na NVIDIA |

Para cada um, anote o fps e se a imagem ficou certa. Mantenha o que ajudou e
some com o próximo (por exemplo: `-Perf -Vrr -Output 1664x936 -More
@{VICTORY_D3D12_BACKGROUND='0'}`).

**Desenhos faltando ou piscando:** a NVIDIA limita os heaps de views a
1.000.000. Em cenas cheias, alguns desenhos podem ficar sem vaga e somem, o que
parece *flicker*, não fps baixo. O log conta isso nas linhas
`asks for dynamic view slots refused`. Mande essas linhas se vir objetos
sumindo.

## 5. Interruptores novos

Todos são opcionais. Sem eles vale o automático descrito acima, e no PC de
desenvolvimento nada muda.

| Interruptor | O que faz |
|---|---|
| `VICTORY_HOST_PRESET=0` | Desliga o perfil automático (dá Series X), os ajustes do launcher (as linhas `host preset`) e os dois do `play.ps1` (prioridades, DRED). A linha `host` e os avisos continuam. O que a própria camada adapta (memória, threads, AVX-512) continua |
| `--console auto\|series-x\|series-s` / `-Console` | Perfil do console; `auto` é o padrão |
| `VICTORY_VRAM_MB=<n>` | A camada trabalha com no máximo `n` MB da placa; `0` usa o orçamento do sistema sem reserva |
| `VICTORY_SMALL_RAM=0\|1` | Força o veredito "PC de pouca RAM" da camada |
| `VICTORY_SHARED_BAND=0\|1` | Força seguir (ou não) o uso da memória compartilhada |
| `VICTORY_LIST_RENEW=0\|1` | Força recriar (ou não) os allocators inchados |
| `VICTORY_TEARING=1` (`-Vrr`) | Swap chain com *tearing* permitido, para VRR. Se o sistema recusar, ela é refeita sem e o log avisa |
| `VICTORY_INDIRECT_WIDE=1` | **Experimental.** Dispatches indiretos de até 2^20 grupos também na NVIDIA. Pode corrigir os blocos no pé da imagem acima de 1440p, mas também pode derrubar o dispositivo |
| `VICTORY_FSR_ANY_PROVIDER=1` | **Experimental, nunca testado.** `-Fsr4K` usa o upscaler mais novo que a DLL oferecer (FSR 3.1 fora do RDNA 4) |
| `VICTORY_SIMD=avx2` | Volta aos kernels AVX2, para comparar com o AVX-512 |
| `VICTORY_DSTORAGE_DECODES=<n>` | Quantas descompressões de DirectStorage ao mesmo tempo (4 aqui; 12 = como antes) |
| `VICTORY_RECORDS_THREADS=<n>` | Threads dos registros do heap (3 aqui; 6 = como antes) |
| `VICTORY_WIDE_ABOVE=0\|1` | Prioridade acima do normal para as threads que a thread de envio espera (1 aqui) |
| `VICTORY_TITLE_PRIORITIES=0\|auto` | `0`: as prioridades do título como ele as define; `auto`: assim só com 16 ou mais threads lógicas |
| `VICTORY_STREAM_PRIORITY=0\|2` | `2`: o streaming sobe de prioridade também quando parte de *idle* (o padrão aqui) |
| `VICTORY_LANE_CORE=1` | Agora também funciona num processador de núcleos iguais: a thread de envio ganha um núcleo só dela |

Exemplo: `scripts\play.ps1 -Exe "..." -Vrr -More @{VICTORY_SIMD='avx2'}`.

Para comparar com o comportamento de antes destas mudanças, o mais perto
possível neste PC:

```powershell
scripts\play.ps1 -Exe "..." -Console series-s -More @{
    VICTORY_HOST_PRESET='0'; VICTORY_SMALL_RAM='0'; VICTORY_SHARED_BAND='0'; VICTORY_VRAM_MB='0'
    VICTORY_LIST_RENEW='0'; VICTORY_SIMD='avx2'; VICTORY_DSTORAGE_DECODES='12'; VICTORY_RECORDS_THREADS='6'
    VICTORY_THIN_THREADS='8'; VICTORY_PIPELINE_WORKERS='8'; VICTORY_WIDE_ABOVE='0'; VICTORY_STREAM_PRIORITY='1'
    VICTORY_CARD_ROOM_FOR_TARGETS_MB='3072'
}
```

Algumas diferenças continuam, e nenhuma delas tem interruptor:

- O piso de texturas e a quantidade de blocos vazios de textura de uma placa
  pequena (orçamento de 12 GB ou menos).
- A segunda tentativa depois de um `E_OUTOFMEMORY`. Antes, a execução terminava
  ali.
- O perfil. Antes, o launcher usava Series X por padrão, com render interno de
  2560×1440. A receita usa Series S de propósito, porque o Series X não cabe em
  8 GB.

## 6. O que não dá nesta placa

- **`-Fsr4K` (FSR 4.1.2):** só roda em Radeon RDNA 4. Numa NVIDIA, o launcher
  recusa logo no início e explica o motivo.
- **Resoluções de render acima de ~2,1 megapixels:** a NVIDIA corta dispatches
  indiretos em 65.535 grupos, o que deixa blocos errados no pé da imagem. O
  launcher avisa quando a resolução passa disso. O Series S (720p interno) fica
  bem abaixo. No Series X, use `VICTORY_OUTPUT=2560x1440` ou `1920x1080`.
- **Series X em 4K** não cabe direito em 8 GB de VRAM.

## 7. O que medir e me mandar

Mande o `run.log`, o `crashes.log` (em `%LOCALAPPDATA%\victory\data`) e:

1. **As linhas do início:** `host`, `console`, todas as `host preset` e os
   `warning`.
2. **A lista do `victory shaders`** de programas recusados ou talvez recusados
   em ondas de 32. Diga também se o log do jogo mostra `refused by a card of
   32-lane waves` ou `host pipeline refused` para algum desses hashes.
3. **Se chegou ao jogo.** Se a execução terminou, diga em que ponto e com qual
   mensagem (falta de memória? *device removed*?).
4. **Memória** num trecho de 15 a 30 minutos, no Gerenciador de Tarefas: RAM em
   uso, "Confirmada" e "Memória dedicada da GPU" / "Memória compartilhada da
   GPU". Mande também as linhas do log `the card's budget: ...` e
   `the PC's memory shared with the card: ...`, e diga com que frequência
   aparecem.
5. **Tempo de quadro** (a taxa aparece na barra de título) numa rua da cidade,
   sempre no mesmo lugar, e a linha do log `the card's clock per frame: nothing
   of ours queued ... (the card about N% busy ...)`. Ela diz se quem segura o
   quadro é a placa (perto de 100 %) ou a thread de envio. Compare com e sem
   `-Perf`, os experimentos do item 4 e cada uma destas variações, uma de cada
   vez:
   - `-More @{VICTORY_SIMD='avx2'}`
   - `-More @{VICTORY_UPSCALER='7'}`
   - `-More @{VICTORY_HOST_PRESET='0'} -Console series-s` (só as escolhas do
     launcher e do `play.ps1` saem)
   - sem `-Vrr`
   - uma execução perto da antiga (abaixo)
6. **Imagem:** blocos ou corrupção no pé da imagem, texturas que demoram a
   aparecer e engasgos de um em um segundo.
7. **Uma mensagem `out of memory (E_OUTOFMEMORY): let go of ...`**, se aparecer,
   e se o jogo continuou depois dela.

Com isso dá para saber o que ajustar em seguida.
