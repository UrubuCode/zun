# Herança única + múltiplas interfaces

Nota de método: afirmações marcadas "(repo)" foram verificadas em arquivos deste repositório. Afirmações marcadas "(C#/ECMA, não verificado aqui)" vêm do conhecimento geral da linguagem/especificação e não foram confirmadas lendo código deste repo.

## Mecanismo no .NET

### Um único pai por tipo

No CoreCLR, o `MethodTable` de uma classe guarda exatamente um ponteiro para o tipo pai:

- `src/coreclr/vm/methodtable.h`, campo `PTR_MethodTable m_pParentMethodTable;` (linha ~3997) e o acessor `GetParentMethodTable()` (linha ~2169). Não existe um "array de pais" para classes (repo).
- O número de slots virtuais herdados é calculado a partir de um único pai: `GetNumParentVirtuals()` em `methodtable.h` (~linha 1793) retorna `pMTParent->GetNumVirtuals()` (ou 0 se não há pai, ou se o tipo é interface) (repo).

Isso significa que o layout de uma instância (campos) e o layout da vtable são construídos por *extensão linear* do pai: os campos do pai vêm primeiro, os do filho depois; os slots virtuais do pai mantêm o mesmo índice no filho, e novos virtuais são apensados ao final. É isso que permite que `obj.Metodo()` virtual seja compilado como "carregue MethodTable, leia slot N, chame" sem conhecer o tipo exato (ver arquivo 02).

### Muitas interfaces por tipo

Interfaces, ao contrário, são uma lista:

- `unsigned GetNumInterfaces()` retorna `m_wNumInterfaces` (`methodtable.h`, ~linha 2123) (repo).
- O mapa de interfaces é um array de `InterfaceInfo_t` (`methodtable.h`, struct na linha ~147) iterado por `MethodTable::InterfaceMapIterator` / `IterateInterfaceMap()` (linhas ~2285-2435) (repo).
- Verificar "este tipo implementa a interface I?" é `MethodTable::ImplementsInterface(MethodTable*)` (linha ~2261), que percorre esse mapa (repo).

Interfaces não contribuem para o layout de campos da instância (interfaces não podem ter campos de instância: ver `docs/design/specs/Ecma-335-Augments.md`, seção "Default Interface Methods": "Interfaces obviously still can't have instance fields") (repo). Elas contribuem apenas com *contratos de métodos*, que são resolvidos por um mecanismo separado (dispatch maps / Virtual Stub Dispatch, arquivo 03).

### Interfaces com implementação padrão

Desde as Default Interface Methods (DIM), interfaces podem ter métodos de instância com corpo. A especificação (repo: `docs/design/specs/Ecma-335-Augments.md`, seção "Default Interface Methods") diz que interfaces agora podem ter métodos de instância virtuais e não virtuais, e que podem fazer `MethodImpl` de métodos de outras interfaces que elas requerem (com o `MethodImpl` obrigatoriamente `final`). O type loader tem lógica própria para isso: `MethodTableBuilder::HasDefaultInterfaceImplementation` em `src/coreclr/vm/methodtablebuilder.cpp` (~linha 11725) (repo).

Isto dá um "herança múltipla de comportamento, sem estado". Quando duas interfaces fornecem implementações padrão conflitantes para o mesmo método, o runtime precisa de uma regra de desempate (a "most specific implementation"); a existência de ambiguidade é um erro de resolução, não algo silenciosamente resolvido. A regra exata está nas Augments do ECMA (repo), mas não reproduzo o algoritmo completo aqui.

### O que o runtime impõe vs. o que a linguagem impõe

- O *runtime* (CLI) permite uma classe com um pai. O formato de metadados (tabela TypeDef) tem um campo `Extends` singular (ECMA-335, não verificado aqui diretamente neste repo).
- A *linguagem* C# adiciona a convenção "classe base opcional + lista de interfaces" (C#/ECMA, não verificado aqui).

## Por quê

Motivos, separando o que está documentado do que é inferência:

1. **Layout de objeto simples e rápido (inferência sustentada pelo código).** Com um pai só, o offset de qualquer campo herdado é fixo e conhecido em tempo de compilação do filho, e o índice de slot de qualquer método virtual herdado é fixo. O `MethodTableBuilder` calcula `bmtVT.cVirtualSlots` por extensão do pai; com dois pais seria necessário ou ajustar `this` em chamadas (thunks, como em C++ com herança múltipla não virtual), ou ter offsets de base virtual (herança virtual). Nada disso existe em `MethodTable` (repo: só há `m_pParentMethodTable`).
2. **Problema do diamante evitado nos dados.** Como interfaces não carregam campos de instância, não há "duas cópias do estado da base comum". O diamante só pode aparecer em *comportamento* (DIM), onde o runtime tem regra explícita.
3. **GC e identidade.** O GC percorre objetos usando o `MethodTable` do objeto (série de ponteiros GC no layout contíguo). Um único layout linear mantém isso simples. (Inferência; o repo documenta o GC em `docs/design/coreclr/botr/garbage-collection.md`, mas não traz a justificativa explícita contra herança múltipla.)
4. **Reflexão, verificação e cast baratos.** `CanCastTo`/`ImplementsInterface` consistem em subir uma cadeia linear (classes) e/ou varrer uma lista (interfaces).

Não encontrei no repositório um documento de design afirmando explicitamente "escolhemos herança única porque X". As razões acima são consequências verificáveis do design do `MethodTable`, não citações de intenção dos autores.

## Trade-offs

| Aspecto | Herança única + interfaces | Herança múltipla de classes |
|---|---|---|
| Layout de instância | Linear, offsets fixos | Precisa de ajuste de `this` ou bases virtuais |
| Vtable | Uma por tipo, estende a do pai | Várias vtables/subobjetos |
| Reuso de implementação | Só via composição, DIM ou helper estático | Direto |
| Diamante de estado | Impossível | Precisa de regra (virtual base) |
| Diamante de comportamento | Possível só com DIM; regra explícita | Possível sempre |
| Custo de dispatch de interface | Mais caro que virtual de classe (arquivo 03) | N/A |

Custos reais no .NET:

- Chamada via interface não é um simples índice de vtable: usa dispatch maps e stubs (`virtualcallstub.cpp`). O doc `docs/design/coreclr/botr/virtual-stub-dispatch.md` descreve o histórico: o mecanismo anterior usava IDs de interface processo-únicos e uma tabela global esparsa, o que prejudicava working set (repo).
- Reuso de código entre hierarquias exige delegação manual (boilerplate) ou DIM. DIM tem limitações (sem campos, e o chamador precisa converter para a interface para acessar o método padrão em C#; esta última parte é C#, não verificada aqui).
- Structs (value types) podem implementar interfaces mas não herdar de classes arbitrárias; convertê-los para interface faz boxing. Isto é um custo conhecido; o repositório tem `constrained.` como mitigação (ver arquivo 05).

## Como aplicar na sua linguagem

Decisões a tomar, com o que o .NET ensina:

1. **Escolha o modelo de pai.** Se quer layout de objeto trivial e vtable linear, adote pai único para tipos com estado. Se aceita custo, pode permitir múltiplos pais mas então precisa definir: ajuste de `this`, ordem de campos, linearização (C3 ou similar) e inicialização das bases.
2. **Separe "tipo com estado" de "contrato".** O .NET manteve a lista de interfaces fora do layout de campos. Você pode ir além: permitir "traits" com estado se aceitar que cada trait ocupa campos próprios do objeto (layout por composição).
3. **Se permitir implementação padrão em interfaces, defina a regra de conflito desde o início.** O .NET tem a noção de "implementação mais específica" e rejeita ambiguidade; incluiu uma checagem de `final` em `MethodImpl` de interface (repo: `methodtablebuilder.cpp`, ~linha 4905: "Non-static interface method body that has methodimpl should always be final").
4. **Decida onde o cast para interface custa.** Se sua linguagem tem structs/valores não-boxed, planeje desde cedo como chamar métodos de interface sem boxing (genéricos reificados + chamada estática resolvida, como `constrained.`).
5. **Armazene interface map por tipo concreto** (lista de interfaces + tabela método-da-interface -> implementação). Isso é o que o `MethodTableBuilder` constrói e o que o dispatch consome.
6. **Considere exportar o contrato, não a cadeia.** Em ABI estável, mudar o pai de uma classe quebra o layout dos filhos (fragile base class). O .NET tem que lidar com isso em ReadyToRun (ver `docs/design/coreclr/botr/readytorun-overview.md`, repo), o que ilustra o custo de expor layout.

Esboço mínimo de representação em runtime para sua linguagem:

```
TypeInfo {
  parent: TypeInfo?            // um só
  interfaces: [InterfaceInfo]  // lista (inclui as herdadas)
  vtable: [CodePtr]            // virtuais de classe (arquivo 02)
  dispatchMap: Map<(Interface, slot), CodePtr>   // interface (arquivo 03)
}
```

## Anexo: exemplo de layout (ilustrativo)

Considere, em pseudo-C#:

```
class A { int x; virtual void F(); }
class B : A { int y; override void F(); virtual void G(); }
interface I { void H(); }
class C : B, I { void H() {...} }
```

Resultado esperado a partir do que o `MethodTable` modela (descrição conceitual, não um dump real do runtime):

- `A`: parent = `System.Object`. Virtuais herdados de `Object` (`ToString`, `Equals`, `GetHashCode`, `Finalize`) vêm primeiro; `F` ganha o próximo índice, digamos N.
- `B`: parent = `A`. `F` reaproveita o slot N (override, sem `newslot`); `G` recebe o slot N+1. Campo `y` fica depois de `x` na instância.
- `C`: parent = `B`. Seu mapa de interfaces contém `I`. A implementação de `I.H` é registrada no dispatch map de `C` (arquivo 03), não na vtable de `B`. `H` pode ou não ser virtual na vtable de `C`: em C#, a implementação implícita de interface é marcada virtual/final no IL (C#/ECMA, não verificado aqui).

Observe a assimetria: o slot de `F` é uma *posição* herdada (vtable linear); o "slot" de `I.H` é uma *chave* (interface, índice) resolvida por mapa. Essa assimetria é o preço de ter uma lista de contratos em vez de uma segunda cadeia de herança.

## Anexo: perguntas de design para sua linguagem

1. Um tipo pode ter dois pais com estado? Se sim, qual é a regra de inicialização e de layout?
2. Interfaces podem ter estado? (No .NET não podem; ver as Augments do ECMA.)
3. Interfaces podem ter código? Se sim, onde fica a regra de conflito e ela é verificada em carga de tipo ou em compilação?
4. A lista de interfaces é fechada no momento da declaração do tipo? (No .NET sim: o interface map é construído em `BuildMethodTable`; extensão posterior não é possível sem reflexão de tipo novo.)
5. Valores não-boxed podem implementar interfaces? Qual o custo ao converter?
6. Mudar o pai ou a lista de interfaces de um tipo publicado é uma quebra binária?

## Resumo dos fatos confirmados

- Classe tem um campo de pai: `m_pParentMethodTable` (`methodtable.h`).
- Tipo tem contagem de interfaces: `m_wNumInterfaces`, acessada por `GetNumInterfaces()`.
- Iteração do mapa: `IterateInterfaceMap()` e `InterfaceMapIterator`.
- Teste de implementação: `ImplementsInterface()` e a variante inline `ImplementsInterfaceInline()`.
- Slots virtuais herdados: `GetNumParentVirtuals()`; interfaces retornam 0.
- Interfaces não têm campos de instância (Augments do ECMA).
- Interfaces podem ter implementação padrão (DIM) e `MethodImpl` final entre interfaces.
- O type loader tem função dedicada a DIM: `HasDefaultInterfaceImplementation`.

## Armadilhas de design observáveis

- Dois caminhos de chamada: virtual de classe (vtable) e de interface (dispatch map). Teste ambos.
- Mudar a lista de interfaces de um tipo muda seu interface map e, portanto, o dispatch map.
- Conversão de struct para interface implica boxing; planeje isso na API.
- Herança de implementação via DIM não traz estado: não espere comportamento "mixin com campos".
- Reuso de estado entre hierarquias exige composição: guarde o componente num campo e delegue.
- Ambiguidade de DIM em diamante é um erro de resolução, não uma escolha silenciosa.
- Em ABI estável, inserir interface em um tipo publicado é uma mudança de contrato.

## Checklist de decisão para a sua linguagem

1. Quantos pais com estado? (recomendação: um)
2. Interfaces/traits podem ter código? (recomendação: sim, sem estado)
3. Quem resolve conflitos de código padrão? (recomendação: regra de "mais específico" e erro se ambíguo)
4. Onde é verificado? (recomendação: na carga do tipo/linkagem, para suportar binários de várias origens)
5. Interface map é por tipo concreto ou global? (o .NET usa por tipo; IDs globais só no token de dispatch)
6. Há restrições sobre interfaces genéricas e variância? (o .NET tem variância; `CanCastTo` precisa tratá-la)
7. Como o tipo de valor implementa interface sem boxing? (ver arquivo 05)
8. O que acontece se o pai é alterado depois de publicado? (fragile base class)

## Referências

- `src/coreclr/vm/methodtable.h`: `m_pParentMethodTable`, `GetParentMethodTable`, `GetNumParentVirtuals`, `GetNumInterfaces`, `InterfaceInfo_t`, `InterfaceMapIterator`, `ImplementsInterface`.
- `src/coreclr/vm/methodtablebuilder.cpp`: construção de vtable e interface map; `HasDefaultInterfaceImplementation` (~11725); regra de `final` para MethodImpl de interface (~4905).
- `docs/design/specs/Ecma-335-Augments.md`: seções "Default Interface Methods" e "Static Interface Methods".
- `docs/design/coreclr/botr/virtual-stub-dispatch.md`: histórico e motivação do dispatch de interfaces.
- `docs/design/coreclr/botr/type-system.md`, `docs/design/coreclr/botr/type-loader.md`: visão geral do carregamento de tipos.
