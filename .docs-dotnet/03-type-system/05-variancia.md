# Variância (in/out) em interfaces e delegates

## Mecanismo no .NET

### 1. Onde a variância é declarada

Variância é um atributo **do parâmetro genérico** no metadado (`gpCovariant`, `gpContravariant`,
`gpNonVariant`). Em C#, `out T` = covariante e `in T` = contravariante. Só **interfaces e
delegates** podem ter parâmetros variantes; isso é codificado nas pré-condições do verificador de
cast: `CanCastByVarianceToInterfaceOrDelegate` exige `pTargetMT->HasVariance()` e
`pTargetMT->IsInterface() || pTargetMT->IsDelegate()`
(`src/coreclr/vm/methodtable.cpp`, ~linha 1207).

No `MethodTable` existe o flag `enum_flag_HasVariance` ("tipo instanciado cujos parâmetros são co- ou
contravariantes", `methodtable.h` ~3756, com `SetHasVariance`/`HasVariance` ~1537-1543). A variância
de cada parâmetro é lida do `EEClass` por `GetVarianceOfTypeParameter(i)`.

### 2. Algoritmo de cast por variância

Trecho real (`methodtable.cpp` ~1207-1290), simplificado:

```
if (this == target) return TRUE;
if (TypeDefRid != || Module != || já visitado) return FALSE;   // mesma definição genérica

for cada argumento i:
    se thArg == thTargetArg  -> continua
    senão switch variância(i):
        covariante     : exige thArg.IsBoxedAndCanCastTo(thTargetArg)
        contravariante : exige thTargetArg.IsBoxedAndCanCastTo(thArg)
        não-variante   : falha
```

Observações confirmadas:
- Só compara instanciações **da mesma definição genérica** (mesmo `TypeDefRid` e módulo).
- Usa uma lista `TypeHandlePairList *pVisited` para quebrar ciclos (ex.: interfaces
  recursivas `IFoo<IFoo<T>>`).
- Há um atalho para um tipo "marcador especial" usado em casting genérico aproximado
  (`IsSpecialMarkerTypeForGenericCasting`).
- O nome `IsBoxedAndCanCastTo` resume a regra importante: se o argumento é tipo de referência, vale
  `CanCastTo`; se é variável genérica, só vale quando está constrangida a ser tipo de referência
  (`ConstrainedAsObjRef`); **qualquer outro caso (value types) retorna FALSE**
  (`src/coreclr/vm/typehandle.cpp`, ~584-620). Portanto `IEnumerable<int>` **não** converte para
  `IEnumerable<object>` apesar de `out T`: a conversão exigiria boxing de cada elemento, que não é
  uma conversão de identidade de representação.

### 3. Por que apenas reference-conversions funcionam

Uma conversão variante precisa ser uma "conversão de identidade de representação": o mesmo ponteiro
serve nos dois tipos. Para referências isso vale. Para value types a representação de `int` e de
`object` difere (struct inline vs ponteiro). O código acima imprime a regra: value type no
argumento → sem variância.

### 4. Variância e despacho de interface

Quando uma classe implementa `IEnumerable<string>` e o código chama pela interface
`IEnumerable<object>`, o runtime precisa achar o slot correto. O Ecma-335 Augments (default
interface methods, `docs/design/specs/Ecma-335-Augments.md` ~linha 432) inclui o passo: "se não
houver método na lista e a interface for variante, repita o algoritmo procurando uma correspondência
variante; retorne a primeira correspondência fornecida por uma interface mais específica".
(O documento `docs/design/coreclr/botr/virtual-stub-dispatch.md` descreve o dispatch de interface em
geral, mas não menciona variância; não afirmamos aqui como os stubs lidam com o caso variante.)

### 5. Covariância de arrays vs variância declarada

Arrays de referência têm covariância **embutida e não-segura** (`string[]` → `object[]`), checada em
runtime em stores (`stelem.ref`, ver helpers de array em `CastHelpers.cs`). É um mecanismo separado
da variância de genéricos declarada, que é estaticamente segura pelas regras de posição
(`out` só em retorno, `in` só em parâmetro — verificado pelo compilador C#, conhecimento geral).

### 6. Covariant return types (outra "covariância")

O runtime também suporta **retorno covariante em overrides** de classes
(`Ecma-335-Augments.md`, seção "Covariant Return Types", ~linha 829; design em
`docs/design/features/covariant-return-methods.md`). Aplica-se apenas a métodos virtuais de classes
(não de interface) via `MethodImpl`; a assinatura é *covariant-return-compatible-with*.

## Por quê

- **Reuso de código tipado**: passar `IEnumerable<string>` onde se espera `IEnumerable<object>`
  sem copiar.
- **Segurança**: restringir a interfaces/delegates com posições `in/out` torna a conversão
  provadamente segura sem checagens por acesso (ao contrário dos arrays).
- **Compatível com reificação**: como `List<string>` e `List<object>` são tipos distintos em runtime,
  precisa-se de uma regra explícita de subtipagem — não existe com erasure (lá a tipagem de
  `? extends`/`? super` é de uso-site e apagada).
- **Limitação a referências** vem da representação: só reference conversions são identidade.

## Trade-offs

- **Complexidade em runtime**: o cast vira um algoritmo recursivo com detecção de ciclo; dispatch de
  interface precisa de caminho variante; sharing de genéricos (`__Canon`) interage com isso, e há
  código especial para "marcador" de casting genérico.
- **Ambiguidade**: um tipo pode implementar `IFoo<A>` e `IFoo<B>` e ser conversível a `IFoo<object>`
  por ambos; o resultado depende da ordem/regra "most specific" (ver `07`, que menciona
  `AmbiguousImplementationException` para DIM).
- **Value types excluídos**: assimetria surpreendente para usuários.
- **Só declarável em interface/delegate**: classes não podem ser variantes (porque têm estado
  mutável em ambas as direções — raciocínio geral).
- **Custo de checagem em cast**: cada cast genérico não-identidade percorre os argumentos.

## Como aplicar na sua linguagem

1. **Variância por declaração (use-site da definição)** é mais simples para usuários que variância
   por uso (wildcards) e permite reificação limpa. Marque parâmetros como `in`/`out` em *interfaces
   e funções/delegates*.
2. **Regra de posições**: o compilador verifica que `out T` só aparece em posições de saída
   (retorno) e `in T` em entrada (parâmetro), transitivamente por interfaces aninhadas.
3. **Limite a conversões de identidade de representação**: se sua linguagem tem tipos de valor
   inline, exclua-os da variância (o runtime .NET faz isso em `IsBoxedAndCanCastTo`).
4. **Implemente o cast variante com visited-set** para evitar recursão infinita.
5. **Decida a relação com dispatch de interface** desde cedo (qual método é chamado quando há
   múltiplas instanciações compatíveis?). Se tiver DIM, adote a regra "interface mais específica"
   e um erro de ambiguidade.
6. **Se tiver arrays covariantes, trate-os como dívida técnica**: o .NET precisa de checagem em
   cada store; um design novo pode simplesmente não oferecer covariância de arrays mutáveis.
7. **Suporte retorno covariante em override** separadamente — é barato e muito útil
   (clonagem, builders).

### Exemplo: o que é permitido e o que não é

```csharp
IEnumerable<string> s = ...;
IEnumerable<object> o = s;        // OK: covariante (out T), string é tipo de referência
IEnumerable<int> i = ...;
// IEnumerable<object> x = i;     // ERRO: int é value type -> sem variância (IsBoxedAndCanCastTo = FALSE)

Action<object> ao = ...;
Action<string> as_ = ao;          // OK: contravariante (in T)
List<string> l = ...;
// List<object> lo = l;           // ERRO: classes não são variantes
```

### Checklist de decisões

- [ ] Variância por declaração (`in`/`out`) ou por uso (wildcards)?
- [ ] Quais construtos aceitam variância (só interface/função)?
- [ ] Value types participam? (.NET: não, só conversões de referência.)
- [ ] Como o cast em runtime evita recursão infinita (visited set)?
- [ ] Como a variância interage com dispatch de interface e com DIM?
- [ ] Arrays são covariantes? Se sim, quem paga a checagem em store?

## Referências

- `src/coreclr/vm/methodtable.cpp` (`CanCastByVarianceToInterfaceOrDelegate`, ~1207)
- `src/coreclr/vm/methodtable.h` (`HasVariance`, `enum_flag_HasVariance`)
- `src/coreclr/vm/typehandle.cpp` (`IsBoxedAndCanCastTo`, `CanCastTo`)
- `docs/design/specs/Ecma-335-Augments.md` (DIM + variância; Covariant Return Types)
- `docs/design/features/covariant-return-methods.md`
- `docs/design/coreclr/botr/virtual-stub-dispatch.md`
