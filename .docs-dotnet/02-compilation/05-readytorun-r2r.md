# ReadyToRun (R2R) — pré-compilação

## Mecanismo no .NET

ReadyToRun é um formato de arquivo onde o **assembly IL original é mantido** (metadata + IL completos) e **código nativo pré-compilado é anexado** ao lado, junto com as tabelas necessárias para o runtime usá-lo (GC info, EH, mapa de métodos, fixups). Nenhuma recompilação é necessária no startup: o runtime encontra o corpo nativo do método e o executa, sem chamar o JIT. Se o código nativo não puder ser usado (versão incompatível, método não pré-compilado, otimização desejada), o runtime volta ao JIT — o R2R é, em essência, **um cache de código** (`readytorun-composite-format-design.md`: "the R2R file is a code cache to save jitting time").

Documentos: `docs/design/coreclr/botr/readytorun-overview.md` (motivação, versionamento), `readytorun-format.md` (formato binário), `docs/design/features/readytorun-composite-format-design.md`, `readytorun-pinvoke.md`, `crossgen2-compilation-structure-enhancements.md`. Estruturas: `src/coreclr/inc/readytorun.h`.

### Por que não é NGEN

O overview é explícito: NGEN era um *cache* — frágil, descartado a cada atualização. R2R carrega *"a strong guarantee that the file will continue to run despite updates and improvements to the runtime or framework."* Consequência: *"anything you place in the file is a liability because you will have to support it in all future runtimes."* Logo, o formato é minimalista.

### Estrutura do arquivo

1. É um assembly CLI normal (arquivo 03), com:
   - plataforma específica;
   - `COMIMAGE_FLAGS_IL_LIBRARY` (0x4) no header CLI;
   - `ManagedNativeHeader` apontando para `READYTORUN_HEADER`.
2. `READYTORUN_HEADER` (`readytorun-format.md`):
   ```c
   struct READYTORUN_HEADER {
       DWORD  Signature;      // 0x00525452 'RTR'
       USHORT MajorVersion, MinorVersion;
       READYTORUN_CORE_HEADER CoreHeader; // Flags, NumberOfSections, READYTORUN_SECTION[]
   };
   ```
   Versão atual no código: `READYTORUN_MAJOR_VERSION 31`, `READYTORUN_MINOR_VERSION 0x0001`, `MINIMUM_READYTORUN_MAJOR_VERSION 26` (`src/coreclr/inc/readytorun.h`). Regra: *major* = quebra de formato; *minor* = mudança compatível ("The runtime should be able to successfully execute native code from images of version 2.9" se suporta 2.3; recusa 3.0).
3. **Seções** (ordenadas por tipo, busca binária; o runtime pode ignorar tipos que não entende):

| Seção | ID | Conteúdo |
|---|---|---|
| CompilerIdentifier | 100 | string ASCII identificando o compilador |
| ImportSections | 101 | slots a preencher fora do módulo (lazy/eager) |
| RuntimeFunctions | 102 | array de `RUNTIME_FUNCTION` (unwind xdata/pdata) + GC info logo após o unwind |
| MethodDefEntryPoints | 103 | array esparso: `MethodDef RID` → índice em RuntimeFunctions + lista de fixups |
| ExceptionInfo | 104 | tabelas EH |
| DebugInfo | 105 | mapas de offsets e variáveis locais (mesma codificação do NGEN; "TODO: document") |
| DelayLoadMethodCallThunks | 106 | thunks de carga tardia |
| AvailableTypes | 108 | tipos disponíveis no módulo |
| InstanceMethodEntryPoints | 109 | entrypoints de instanciações genéricas (hashtable) |
| InliningInfo / InliningInfo2 | 110 / 114 | quem inlinou quem (para rejit/profiler/versioning) |
| ManifestMetadata | 112 | metadata extra de assembly refs |
| ComponentAssemblies / OwnerCompositeExecutable | 115 / 116 | modo composto |
| PgoInstrumentationData | 117 | dados de PGO |
| CrossModuleInlineInfo | 119 | inlining cross-módulo |
| HotColdMap | 120 | hot/cold splitting |
| MethodIsGenericMap / EnclosingTypeMap / TypeGenericInfoMap | 121–123 | mapas v9 |
| ExternalTypeMaps / ProxyTypeMaps / TypeMapAssemblyTargets | 124–126 | type maps |

4. **Flags** (`READYTORUN_FLAG_*`): `PLATFORM_NEUTRAL_SOURCE=0x1`, `COMPOSITE=0x2`, `PARTIAL=0x4`, `NONSHARED_PINVOKE_STUBS=0x8`, `EMBEDDED_MSIL=0x10`, `COMPONENT=0x20`, `MULTIMODULE_VERSION_BUBBLE=0x40`, `UNRELATED_R2R_CODE=0x80`, `PLATFORM_NATIVE_IMAGE=0x100`, `SINGLE_THREADED=0x2000` (o runtime rejeita a imagem se o modelo de threading não bate).

### Formato nativo de dados ("Native Format")

Tabelas auxiliares usam o "native format" (herdado do .NET Native): inteiros de tamanho variável cujo bit baixo-ordem codifica quantos bytes seguem (`xxxxxxx0`=1 byte, `xxxxxx01`=2, `xxxxx011`=3, `xxxx0111`=4, `xxxx1111`=5), **sparse arrays** e **hashtables** (`readytorun-format.md`, "Native Format"). A lista de fixups de um método usa nibbles de 3 bits de valor + 1 de continuação, com deltas e terminador 0.

### Como o código pré-compilado é "resiliente a versão"

O desafio do R2R é produzir nativo que continue correto se **outros** assemblies (o framework) mudarem. O overview define:

- **Version bubble**: *"a set of DLLs that we are willing to update as a set"*. Dentro da bolha, inlining e otimizações cross-módulo são permitidas; fora, não. *"Code of methods and types that do NOT span version bubbles does NOT pay a performance penalty."* No extremo, a aplicação inteira é uma bolha.
- **Regra nova de compatibilidade**: mudar o número/tipo de campos (inclusive privados) de um struct público é *breaking change*. Structs não públicos e inacessíveis por campos de structs públicos estão isentos.
- **Layouts fixos** para `System.Object`, arrays e `System.String` (primeiro campo = ponteiro de tipo; arrays: depois o length; strings: int32 length + UTF-16 terminada em zero).
- **Acesso a campo** (`ldfld`): se a hierarquia está dentro da bolha, offset constante; caso contrário o compilador emite `MOV TMP,[SIZE_OF_BASECLASS]; MOV EAX,[RCX+TMP+subfield_offset]`, com um slot `SIZE_OF_BASECLASS` preenchido no load do módulo. Custo medido: < 1% num loop contrived. Há ainda restrição de crescimento da base (limite de null-check por página 64K).
- **Chamadas não virtuais** entre módulos: `CALL [PTR_IMETHOD]` — slot resolvido por *delay load* com `RUNTIME_ENTRY_FIXUP_METHOD`. É "a baseline solution to all other versioning issues".
- **Interfaces**: *Virtual Stub Dispatch* — um slot por **call site**; já é resiliente a versão, sem penalidade. Dispatch via vtable cruzando bolhas não é usado (se usa VSD).
- **`newobj`/`newarr`/`isinst`/`castclass`**: chamada de helper indireta (`CALL [NEWOBJ_SUBCLASS]`), com slot por tipo; o runtime escolhe o helper.
- **GC info de tipos / MethodTable**: construídos em runtime a partir da metadata ("generating method tables at runtime will solve a number of other versioning issues").
- **Inlining cross-bubble**: proibido por padrão, exceto métodos com `NonVersionableAttribute` (autor desistiu do direito de mudar o corpo). *"Experiments with disabled cross-module inlining with the selectively enabled inlining of critical methods showed no visible regression in ASP.NET throughput."*
- **Uso seletivo do JIT**: *"selective use of the JIT compiler ... avoid code quality penalties"* — um método quente que cruza bolhas pode ser marcado para ser compilado em runtime.

### Fixups: ligação preguiçosa

`ImportSections` descreve intervalos de slots (`READYTORUN_IMPORT_SECTION`: seção, flags `Eager`=0x1/`PCode`=0x4, tipo `StubDispatch`=2/`StringHandle`=3/`ILBodyFixups`=7, tamanho da entrada, RVA de assinaturas). Cada slot tem uma **assinatura de fixup** cujo primeiro byte é o *kind*, por exemplo `READYTORUN_FIXUP_TypeHandle=0x10`, `MethodHandle=0x11`, `FieldHandle=0x12`, `MethodEntry=0x13`, `MethodEntry_DefToken=0x14`, `VirtualEntry=0x16`, `ThisObjDictionaryLookup=0x07`, `TypeDictionaryLookup=0x08`, `MethodDictionaryLookup=0x09`, `Check_IL_Body=0x35`, `Verify_IL_Body=0x36`, `ModuleOverride=0x80`. Fixups de verificação (`Verify_VirtualFunctionOverride=0x34`, `Check_IL_Body`) garantem em runtime que as premissas do compilador ainda valem; se falhar, o código é descartado (ou falha dura).

### Interação com tiering

`docs/design/features/tiered-compilation.md`: *"For methods that are precompiled in ReadyToRun images, the precompiled code is the tier0 version."* Métodos R2R quentes (30 chamadas + timer de 100ms de startup, nos números do doc 2.1) são recompilados em tier1 pelo JIT, que vê as dependências carregadas e o conjunto de instruções da CPU. Por isso R2R é complemento ao JIT, não substituto.

### Variantes

- **Composite**: vários assemblies em um só binário R2R (`READYTORUN_FLAG_COMPOSITE`), localizado pelo símbolo exportado `RTR_HEADER` (customizável com `--rtr-header-symbol-name`). Metadata/IL podem ser embutidos (`EMBEDDED_MSIL`) ou ficar em arquivos separados reescritos com um header R2R que aponta ao dono (`OwnerCompositeExecutable`).
- **Envelope nativo**: planejado Mach-O para composite no .NET 11 (`readytorun-platform-native-envelope.md`).
- **P/Invoke pré-gerado**: stubs de marshalling gerados em tempo de compilação para marshallers comuns (primitivos, blittable structs, ponteiros, arrays de primitivos, strings ANSI/Unicode/UTF-8, SafeHandle, delegates); os demais ficam JIT (`readytorun-pinvoke.md`).
- **PerfMap**: arquivo `.r2rmap` (texto UTF-8; entradas RVA/tamanho/nome) casado por Debug Directory tipo 21 (`r2r-perfmap-format.md`, `PE-COFF.md`).

### Limitações conhecidas no próprio doc

"Type loading from IL metadata" (tipos são sempre construídos da metadata em runtime — impede stripping), "Debug info size" (infos de debug embutidas inflam a imagem).

## Por quê

- **Startup**: eliminar JIT no caminho de inicialização ("Native code generation takes a relatively long time and consumes power").
- **Integridade/assinatura**: poder validar o código nativo distribuído.
- **Resiliência a atualizações**: ao contrário do NGEN, o arquivo continua a funcionar após patches do framework.
- **Paridade com código nativo**: o overview coloca como meta dar ao gerenciado características de deploy do nativo — formato executável diretamente, atualizações independentes, formato público e documentado.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Manter IL + metadata completos | Todos os recursos do runtime funcionam; fallback ao JIT | Imagem maior; sem stripping |
| Proibir inlining cross-bubble | Resiliência a versão | Código menos otimizado (mitigado: `NonVersionable`, bolhas maiores, tiering) |
| Indireção por slots/fixups | Resolução lazy, versão-segura | Chamadas indiretas e código maior; trabalho no load |
| Struct layout congelado | Código eficiente para structs | Nova regra de compatibilidade para autores |
| Layouts fixos de Object/Array/String | Acesso rápido | Runtime não pode evoluir esses layouts |
| Bolha grande/composite | Melhor performance, deploy simples | Todos os componentes devem ser atualizados juntos; "FailFast" se regras de bolha violadas (crossgen2-compilation-structure) |
| Formato versionado major/minor | Runtime novo roda imagens antigas | Runtime carrega suporte a formatos antigos para sempre |
| Qualidade inferior ao tier1 | Startup rápido | Precisa de tiering para pico de desempenho |

## Como aplicar na sua linguagem

1. **Trate o código pré-compilado como cache sobre um artefato canônico (IL/bytecode)**. Mantenha sempre o fallback para interpretação/JIT; isso simplifica muito as garantias.
2. **Defina "version bubble" explicitamente**: o conjunto de módulos que podem ser otimizados juntos. Escolha o que cruza fronteiras (chamadas, campos, alocação) e use *slots* indiretos preenchidos preguiçosamente.
3. **Congele o que é caro de indireção** (layout de objeto base, string, array; layout de structs públicos) e documente como regra de compatibilidade da linguagem.
4. **Cabeçalho com assinatura mágica + major/minor** e **seção-tabela ordenada** que o runtime pode ignorar por tipo. Regra: runtime aceita minor maior, recusa major maior.
5. **Fixups tipados** (kind + assinatura) em vez de relocations opacas, para que o runtime possa resolver, validar e (em nova versão) substituir por outro helper mantendo o antigo.
6. **Fixups de verificação** (`Check_IL_Body`): código gerado sob premissa deve poder ser invalidado em runtime sem crash.
7. **Dois estágios (tier0 = pré-compilado, tier1 = JIT)** dão ao usuário startup rápido sem sacrificar pico.
8. **Guarde um mapa "método → código + fixups"** num array esparso indexado pelo ID de método (RID), com varint.
9. **Pense em debug/profiling cedo**: perfmap, debug directory e mapas IL↔nativo precisam existir para ferramentas (arquivo 09).
10. **Decida o envelope** (PE/ELF/Mach-O) com antecedência; migrar depois (como o .NET planeja) dá trabalho.

## Referências

- `docs/design/coreclr/botr/readytorun-overview.md` — motivação, version bubbles, codegen resiliente por instrução.
- `docs/design/coreclr/botr/readytorun-format.md` — header, seções, fixups, native format.
- `docs/design/coreclr/botr/readytorun-platform-native-envelope.md`, `r2r-perfmap-format.md`.
- `docs/design/features/readytorun-composite-format-design.md`, `readytorun-pinvoke.md`, `crossgen2-compilation-structure-enhancements.md`, `tiered-compilation.md`.
- `src/coreclr/inc/readytorun.h` — constantes, versões, helpers (`READYTORUN_HELPER_*`).
- `src/coreclr/tools/Common/Internal/Runtime/ModuleHeaders.cs` — espelho gerenciado das constantes.
- `src/coreclr/tools/aot/ILCompiler.ReadyToRun/` — compilador (arquivo 06); `src/coreclr/tools/aot/ILCompiler.Reflection.ReadyToRun/` — leitor.
- ECMA-335 Partição II §25 (formato PE/CLI), pois R2R é extensão via `ManagedNativeHeader`.

## Apêndice A — Ciclo de vida de uma chamada em código R2R

1. **Carga do módulo.** O runtime reconhece a imagem R2R (`HasReadyToRunHeader`), verifica `MajorVersion` (≥ `MINIMUM_READYTORUN_MAJOR_VERSION` = 26 e compatível com `READYTORUN_MAJOR_VERSION` = 31 no código atual) e flags (ex.: `SINGLE_THREADED` precisa casar com o runtime). Seções de import marcadas `Eager` são resolvidas já neste momento.
2. **Primeira chamada a um método.** O prestub consulta `MethodDefEntryPoints` (array esparso indexado por `MethodDef RID`). Se há entrada, obtém o índice em `RuntimeFunctions` (início do código + unwind + GC info) e a **lista de fixups** que precisam estar resolvidos antes de executar.
3. **Fixups.** Para cada slot da lista, o runtime lê a assinatura (`READYTORUN_FIXUP_*`), resolve (tipo, método, campo, dicionário genérico...) e grava no slot. Alguns são *lazy* e resolvidos pelo primeiro uso (`DelayLoadMethodCallThunks`).
4. **Execução.** O código nativo executa; chamadas entre módulos são `call [slot]`; novas alocações chamam helpers via slots; interfaces usam *stub dispatch*.
5. **Tiering.** Se o método for quente (contador + atraso de startup), o JIT gera uma versão tier1 e o `CodeVersionManager` troca o ponto de entrada (`code-versioning.md`).
6. **Rejeição.** Se um fixup de verificação falhar (`Check_IL_Body`, `Verify_VirtualFunctionOverride`), o código R2R daquele método é descartado e o JIT assume (ou o processo falha, no caso dos `Verify_*`).

## Apêndice B — Roteiro mínimo para implementar um "R2R" na sua linguagem

1. Escolha o artefato canônico (bytecode + metadata) e um **header de extensão** apontado pelo header principal.
2. Gere, por método: bytes de código, tabela de realocação **como lista de fixups tipados**, mapa de GC/pontos seguros, tabela de exceções e informação de unwind.
3. Gere um **índice método→código** (`MethodDefEntryPoints`) com varint e array esparso; escolha uma codificação para a lista de fixups (nibbles com continuação + deltas, como no doc).
4. Forneça o **loader**: validar versão, aplicar fixups sob demanda, cair para JIT/interpretador.
5. Defina **regras de compatibilidade** (o que o autor da biblioteca não pode mudar sem quebrar clientes pré-compilados): no .NET, layout de structs públicos, layout de `Object`/array/`String`.
6. Faça a **política de inlining** dependente de uma noção de bolha de versão (`CompilationModuleGroup`) e ofereça um atributo de exceção (`NonVersionable`).
7. Teste com **mudanças reais de biblioteca** (adicionar campo a classe, adicionar método virtual, mudar corpo) e confirme que as imagens antigas continuam corretas — esse é o "contrato" do R2R.

## Apêndice C — Perguntas frequentes derivadas dos docs

- *O R2R substitui o JIT?* Não: ele mantém o JIT para métodos não pré-compilados, genéricos fora da bolha e rejit de tier1.
- *Por que não colocar o `MethodTable` pronto no arquivo?* O doc declara como limitação atual: "All types are built from IL metadata at runtime currently", e propõe como melhoria futura uma seção de layout compacto para carga de tipos.
- *Por que PE em Linux?* Decisão histórica: "Up through .NET 10, ReadyToRun (R2R) uses the Windows PE format as the native envelope on every platform"; o plano para .NET 11 é suportar Mach-O em composite.
- *O que é o `RTR_HEADER`?* Símbolo exportado que aponta o `READYTORUN_HEADER` de imagens compostas (customizável por `--rtr-header-symbol-name` para hosts que linkam várias imagens diretamente).
