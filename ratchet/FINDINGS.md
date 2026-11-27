# Ratchet findings

What the measurements have shown so far. `log.tsv` holds every evaluation and its verdict, and [standings.md](standings.md) every program beside its C twin. The reports the log names are in `ratchet/reports/` of the main checkout (gitignored); the counts are exact, so running the same two revisions again writes the same numbers.

## Where Faun stands against C

At `c3cded9`, as geometric means over the programs that have a twin on the target:

| Target, lab backend | Speed | Memory | Size | Small |
| ------------------- | ----- | ------ | ---- | ----- |
| x86-64 Linux        | 6.9   | 0.16   | 1.8  | 1.3   |
| wasm32 WASI         | 44    | 5.7    | 1.9  | 1.2   |
| 6502 sim65          | 20    | 7.4    | 13   | 13    |

- **x86-64** runs within about twice C's instructions on six programs (binary-trees, checksum, mandelbrot, spectral-norm, fasta, n-body) and holds a sixth of its memory, a static image against one that loads a C library.
- **Lab against stock:** on x86-64 lab runs 12% fewer instructions, holds 13% less and is 19% smaller. binary-trees halves in instructions and memory, and mandelbrot runs 37% fewer. On wasm32 and the 6502 lab is 7 to 9% smaller and otherwise within a few percent of stock.

## The largest gaps

- **Floats on wasm32 are in software.** The target says it computes no floats, so mandelbrot, n-body and spectral-norm burn 6,600 to 24,000 times the twin's fuel. Each module that reaches floats, k-nucleotide's among them, is over 300 KB where the rest are 6 to 65 KB. wasm32 has float instructions, so answering the float claims with them is the largest single gain the suite can see.
- **Plain loops over naturals and lists.** collatz runs 212 instructions a step on x86-64 where C runs 5: 44 times C there, 31 on wasm32 and 13 on the 6502. queens and sieve are 36 times C on x86-64, and pidigits 21 times GMP.
- **Lists rebuilt and `Map`:** fannkuch-redux is 57 times C on x86-64 and k-nucleotide 58, as the benchmarks game's README explains.
- **Memory on wasm32.** Every program touches at least 72 KiB of linear memory where a C twin touches 12 to 16, and checksum holds its whole megabyte of input where C reads a byte at a time.
- **The 6502's images.** The smallest program, checksum, is 21 KB where cc65's is 2.6 KB, and those that fit are 13 times their twins. Six programs fit in 64 KiB on neither backend, MD5 does not fit on lab, and binary-trees and sort do not fit on stock. On lab, binary-trees overflows its stack at its smallest depth, and sort does by 50 numbers.
- **A compiler error.** Stock for the 6502 refuses MD5 with `after natural legalization: main! instruction follows control flow that does not come back`, which is no size limit.

## What merging the lab backend changed

`c3cded9` against `711f676`, the commit before the merge, on the stock backend both have:

- **x86-64 mandelbrot regressed.** It runs 17.4% more instructions, 143.2 M from 122.0 M at size 400, and takes 10 to 20% more processor time, the edge of what the time guard reports. The gate before this one compared lab with lab and so did not see the default backend.
- **6502 checksum regressed.** It takes 2.7 times the cycles, 11.0 M from 4.1 M, and is 2.7% larger: the constant rotation the lab's README records.
- **Gains:** wasm32 MD5 burns 60% less fuel and mandelbrot 13% less, x86-64 k-nucleotide runs 5% fewer instructions, and the float programs' images are 3 to 6% smaller on wasm32 and the 6502.

Under these rules the merge is a rejection for the two regressions, each past 10% in its cell.

## What the code generation series showed

Series 2, from `4a66a65`, aimed at word-heavy hot loops like the
compression libraries'. Two candidates were accepted: natural comparisons
asked as one word comparison (`16405ac`, score 0.992), and machine IR
folding a known shift, conversion, comparison and branch (`7809e77`, score
0.963).

- **A change on x86-64 alone rarely clears the neutral line.** x86-64
  speed is one group of thirteen, so a change that only moves it has to
  take about 1.3% off every x86-64 program, on both backends, to move the
  score by a tenth of a percent. Threading an edge that carries a number
  past a block that only tests it, so `a & b` branches on its comparisons,
  took 10% off deflate's instructions at level 9 and changed no cell for
  the worse, and scored 0.99916: neutral, four times over, with and
  without moving a branch's comparison down beside it. The suite's loops
  have few conditions joined by `&` or `|`; the compression libraries are
  full of them. It is kept on the branch `ratchet/codegen-threading`.
- **The same for inlining.** Copying a definition that costs 24 or less
  wherever it is called, past the caller's allowance, inlines `U64.add`
  into inflate's `codes` and takes 10% off inflate's instructions, but the
  suite's programs are small enough that the allowance never runs out:
  neutral, 0.9993, with 6502 images up to 1% larger.
- **Size pays for a push and a pop.** Writing a small constant as one move
  rather than a push and a pop, favouring speed, ran 0.9% fewer
  instructions across the targets but made x86-64 images 1.5% larger, and
  0.9% larger written only in blocks on a cycle: rejected both ways, the
  loss more than half the gain.
- **What changes many targets at once is what moves the score.** Both
  accepted candidates are in machine IR, which every target reads: the
  comparison took 14 to 17% off sieve and 8 to 12% off queens on x86-64
  and wasm32, and the folding 17% off collatz on x86-64, 65% off checksum
  on the 6502 and 4 to 10% off every image.
- **The 6502 lab sort is at the edge of its memory.** Its image is 63 KB
  of 64, so its bytes written move by tens of bytes with nothing but the
  length of the name it is run under, which the evaluator holds fixed. A
  wider fold, following a number through any conversion to a word, made it
  write 1,508 bytes rather than 804 at 24 numbers, and 1,345 rather than
  815 at none, with the same output and every frame the same size; four
  string appends whose separator's length was known became unconditional,
  and nothing else changed. Following a number to a word only from an
  address, the handle a natural constant is, kept collatz's gain and left
  sort within the 6502's slack. The cause on the 6502 was not found.
- **A cached incumbent compiler can be stale.** The evaluator exports each
  revision into one tree per commit but builds all of them in one shared
  target directory, and `git archive` gives every file its commit's time,
  so cargo can take an older export's build as fresh: `16405ac`'s cached
  compiler was the trunk's, and a candidate on top of it was credited
  with `16405ac`'s own gain again (logged `invalid`). Deleting
  `~/.cache/faun-ratchet/bin/<commit>` and touching the export's files
  rebuilt it. The evaluator is protected, so this is for a pull request of
  its own.

## About the measures

- **Resident memory from `wait4` is rounded** to a quarter of a megabyte on this 32-core machine: fasta reads 512 KiB where it holds 96. `eval/measure.c` counts the page tables instead, and ten passes of every Faun cell agreed to the kilobyte.
- **Elapsed time is not usable on a busy machine:** with every core loaded it reported programs 50 to 80% slower whose instructions had not changed. So did processor time, by its median alone and by its quickest run alone. Asking both of interleaved runs reported only mandelbrot, six timings of six.

- **A memory past the guard is measured again at three quarters of its size** from series 6. Octagons' x86-64 lab collatz holds 30.8% more at 20,000 and 8.6% less at 15,000, and is noted rather than rejected; the 6502 lab sort of the packed fields writes 1,502 bytes to 804 at 24 numbers and 1,502 to 756 at 18, and still rejects.
- **The 6502 sort's first arena is not only what is written.** Counting a byte as held from when it is written until it is last read, in sim65 (not committed), the trunk before the packed fields holds 242 bytes at most at 24 numbers and the packed fields 528, 218 and 450 of them in the heap: next-fit reads every block's header as it passes, so a free block in the 1 KiB arena keeps its header live until the search comes round. Neither the bytes written nor the bytes read again tell this from a program holding more; only the allocator knows which blocks are free, so a measure that passes it is the heap counting what it has handed out, a compiler change, or a first arena asked for by what is needed.
- **A loop's alignment moves a time with the instructions the same.** Octagons' x86-64 lab checksum runs 17.7% longer timed on a busy CPU with 944 fewer instructions in 94 million and as many branches. From series 6 a time is noted, not rejected, where the instructions and the branches are within a tenth of a percent and the memory is the same.

## Lists built only to be walked or compared

- **Fuel weighs a buffer lightly.** The Barb interpreter the fuel tests run on charged `List.range(48).fold` 12% more than the loop by hand, where x86-64 ran 3.4 times the instructions and 250 times the memory for `List.range(n).fold` at three million. A fuel test of something left unbuilt compares against the hand-written loop within a few percent, not against a ratio.
- **A 6502 image near 64 KiB leaves the heap no room.** Fusing queens' range but not the copy of its chain inside the fused step grew the stock image from 61.8 to 63.9 KB, and the program ran out of heap at its smallest size, status 2 after 274 thousand cycles. Fusing the chain inside the step too took it to 52 KB.
- **Constant evaluation is quadratic in a list built at its end.** A repeat built front to back appends a walk at a time when Barb tries to fold it, and trying `List.repeat(0, 1024)` cost inflate 31% more compiling before giving up. A walk pushing one value every round is the same list from either end, so repeat stays built from the end and fusion takes it as it is. Answering a recognized append in the evaluator directly made the attempts go further, not stop sooner: 9% more compiling with the old library.
- **The suite compares no literal list.** Following any field-less known value as a construction, `False` or `[]`, copied walks across the suite for nothing, 0.5 to 1% larger and a 6502 sort past its stack; md5's four-word digest through `flat_map` unrolled into four copies, 6% larger. Only a construction built in a body run more than once is followed now, within what inlining one callee may copy, and the suite's images are byte for byte the same.

## Small tags and pairs packed in a list's elements

- **The suite holds no list of small tags.** Packing a small sum's or pair's fields by their bits in the elements of a list built as the program runs, a block's symbols four bytes apart rather than six, changed no measure of the suite by a tenth of a percent: the win is on the flate probe, where `List([Literal(U8), Match(Nat, Nat)])` held 60 MB and now holds the 40 MB the packed-word workaround did, and ran 758 million instructions, now 743, against the workaround's 689.
- **Giving up and claiming a run of elements in one routine each** is where the series' gains came from: fannkuch-redux 27% fewer instructions on x86-64 and 22% less fuel on wasm32, inflate 14% fewer, and the 6502 lab binary-trees fitting the machine, since one walk called from every release and copy is smaller than one written into each.
- **Shrinking a 6502 image can raise its memory.** The 6502 lab sort image is 63,166 bytes on the trunk and 544 smaller with the routines above. On the trunk the heap's first request, the 1 KiB first arena, would reach the frames, so `grow` refuses it and the heap grows a block at a time exactly to what is held, 498 bytes. With the smaller image the 1 KiB arena fits, and the allocator, searching on from where the last allocation ended, writes through all of it before coming back: 1,022 bytes written at every size from 4 numbers to 32, with the same output, so 87% more memory by bytes written and the candidate rejected. Asking for a 512-byte first arena instead writes 1,243. Any change shrinking the sort image by some 400 bytes trips the same, until the 6502's heap asks for its first arena by what it needs or a search starts where blocks were freed.

## Range analysis as octagons

- **The lab collatz memory is where the heap's next arena falls.** With a
  length bounded by memory or octagons in loops, `List.range(limit)`'s
  elements are `0..2^47` rather than naturals, and the x86-64 lab collatz
  holds 488 KiB at 20,000 rather than 376, measured by `eval/measure.c`.
  The program is the same but for that element class, and which side holds
  more turns over with the size: candidate against incumbent, it is 160 against 132 KiB at 5,000, 268 against 212
  at 10,000, 308 against 336 at 15,000, 488 against 376 at 20,000, 564
  against 616 at 30,000, 920 against 696 at 40,000 and 1,076 against 1,176
  at 60,000, so a measurement artifact rather than a cost the change
  carries. `strace` shows why: at 20,000 the candidate maps a fourth
  arena of 512 KiB that the incumbent's buffers, grown by doubling, happen
  to fit without, and at 15,000 both map three. Like the 6502 sort's first
  arena, a heap growing an arena at a time measures where a buffer's
  doubling lands against an arena's end, not what a program holds; the
  rule's 16 KiB of slack does not cover a 512 KiB arena.

## The 6502 heap by what it holds

- **A 16-bit heap now grows by a quarter of itself, from what it is first asked.** It counts itself as 64 bytes to start and asks `grow` for a quarter of what it has, in whole words, or what the request wants, where it took a 1 KiB first arena and doubled. Next-fit still writes through everything it has before reusing what was freed, so the bytes written are now about a quarter over what the program held at once. Across the 6502 cells of the suite and the probes, bytes written fall 46% by geometric mean and cycles rise 2.0%, at most 4.7% (queens); every image is one byte larger, and none starts or stops fitting. Scored, the suite's 6502 memory is 36% less on stock and 40% on lab for 0.8% and 2.0% more cycles, and the probes' 49% less for 2.3% more, worst cell queens on stock at +4.0% cycles: score 0.960, accepted.
- **The sort the first arena tripped on.** The 6502 lab sort writes 463, 567, 690, 824, 890, 892, 1,241 and 1,244 bytes at 4, 8, 16, 18, 24, 32, 40 and 48 numbers, where the trunk wrote 1,321 to 1,556, the 1 KiB arena showing at every size; cycles are 3 to 9% more. An image shrinking by 544 bytes no longer moves its memory by an arena.
- **A heap block must be whole words.** A quarter of a heap counted in even bytes is odd one time in two; blocks of an odd size ran on into their neighbours' headers on the 6502, so `natural_words` printed nonsense at a 64-byte start and not at 1,024 or at doubling.
- **Taking the lowest block freed since the last allocation first** — `release` noting it, `allocate` claiming it where it fits, the rover left where it was — cut the 6502's cycles 10 to 28% and its memory a little further, but made every image 459 bytes larger (a 16-bit compare and branch costs some ten bytes), and binary-trees on stock and sort on lab, images within 3 KB of the frames, then ran out of room. Starting the search there instead was 210 bytes and cost cycles: from a hole the search walks every live block above it. Either wants the image room first; the reuse is the next gain once it is cheap in bytes.

## Inflate against its twin

Toward inflate within four times its C twin on x86-64 at 100 KB. It started at 21.5 times C, 404 million instructions against 18.8 million.

- **Where the time went, by phase,** each measured by stopping the Faun program and its C twin after it: building the text took 135 million instructions against 1.1 million, compressing 147 million against 12.9, inflating 98 million against 4.3, and the CRC 3.7 million against 0.6.
- **The text was rebuilt every word.** Barb holds a value every caller passes alike (`hold_shared`), so the list of sixteen words became a constant read inside the loop, and lowering builds a list of strings where it is read. Machine IR now builds a construction from constants alone once before the loop holding it and carries it round (`ir/hoist.rs`): the text takes 7.8 million instructions, and inflate runs a third fewer on x86-64, 13.6 times C.
- **Naturals nothing bounds cost three quarters of inflating.** The bit position is compared with no length, since a read past the input answers nought, so it stays a natural through `code_bits`, `bits` and the symbol loop. With the decoder's positions and codes written as `U64` in a scratch copy, the whole program runs 182 million instructions where it ran 256; inflating alone goes from about 100 million to 25, 5.8 times C.
- **Dividing a natural by a constant power of two was a hardware division.** A natural moved down instead, with the immediate case written in place, measured nothing on inflate (258 million against 256) or on PNG decoding (3.47 billion against 3.46), so it was not kept.
- **Range versioning works by hand.** A walk whose recursive call checks its natural is below 2^61, and goes to an unchecked copy where it is not, gets its parameter as a word, since a parameter's bound is what its callers hand it: `code_bits` written so takes its position as `0..2^61` and the program runs 243 million instructions where it ran 256. Done everywhere a walk carries a natural nothing bounds, that is the 182 million the `U64` copy shows, for a copy of each such walk.
- **x86-64 keeps five registers across calls,** `r12` to `r15` and `rbp`. A loop holding more than that across a call on a cold path spills, and the allocator, walking each block in order, can leave the spill on the hot path as well.
- **What the instructions are for.** Sorting every sampled instruction of the workload by what it does, at the hoisting: register-to-register moves 25%, arithmetic 19%, stack spills and reloads 18%, comparisons 13%, branches 12%, natural tag tests 7%, calls 3%, other memory 2%, and counting claims written in place 2%. Counting claims, with the runtime's calls, is some 5% once the text is no longer rebuilt; the moves and spills, 43%, are the work that need not be there.
- **A sum copied to keep its operand.** An x86 sum writes over its first operand, so one whose operand is read after was a copy and a sum, a fifth of the moves. A full-width sum now answers in any register, written as `lea` where that is not its operand's: inflate runs 248 million instructions where it ran 256 and 2.6% fewer cycles, PNG decoding 2.7% fewer instructions and 1.1% fewer cycles, and the walk probes 8.6% fewer instructions. A rewrite of a copy and the sum after it, once allocated, never matched: the copy keeps the operand and the sum stays in its register, so only allocation can choose otherwise.
- **A small number pushed and popped.** Writing one by a move rather than `push`/`pop` took inflate's instructions 1.4% down and its cycles not at all, and made every image larger, so it was not kept: it would move the count and not the program.
- **Cycles against instructions.** At 248 million instructions inflate runs 45.5 million cycles, against the C twin's 18.9 million instructions and 6.8 million cycles: 13 times C in instructions but 6.7 in cycles, Faun's simpler instructions running more to a cycle.
- **Claims as instructions.** Where a claim is taken and given up is now decided by a pass of its own, `ir/claims.rs`, which writes `Claim` and `Release` into machine IR; legalization only writes the count each asks for. With nothing else changed, every image built favouring speed on x86-64 and wasm32 is the same bytes as before. Builds counting claims by calls, every `--small` build and the 6502, differ only in the order of independent instructions, a tenth of a percent either way. Region planning and the check below a power of two had to read a claim as neither publishing a cell nor making an arm more than scalar work; until the second did, mandelbrot ran 6.6% more instructions. `faun emit claims` prints a program with its claims placed.
- **Claims that count nothing, taken out.** Every claim to a natural no wider than half a word goes, by the bits its instructions bound, and to one laid out before the program runs. A loop parameter every way round hands on as it came, and nothing in the loop takes, is borrowed, so a function handing such a loop its parameter borrows it too, and its callers take no claim for the call. A natural sum of a natural and two small numbers adds the two in the word first. The inflate workload's claims fall from 31 taken and 261 given up to 15 and 168; the score against the trunk goes from 0.995 to 0.972, the chain probes 17% faster and smaller, inflate 12.9 times C. Worked out per loop as it was asked, which loops borrow cost the wasm32 float programs up to 71% more compile time, and was rejected until it was worked out once.
- **PNG decoding ran 0.9% more instructions** with borrowed loops though inflating ran 1.3% fewer: the claims placed fall in every function but `straight_pixels`, which gives one up after each call to a reader that now borrows, and the rise is in the unfiltering walks; not yet explained.
- **A condition asked twice in a row was worked out into a register.** Lowering writes a branch for each value an arm answers, so a read that can miss asks its condition two or three times in a row: 23 of the inflate workload's 173 branches and 151 of PNG decoding's 1,363 sit beside one on the same condition. A condition read by more than one branch is not fused with its jump, so each was `sete`, `movzbl`, `test`, `je`. Target preparation now merges each such run once fitting has said how many values a branch carries, so the 6502 keeps branches answering at most two. Answering everything every branch of the run answered first made the tally probes 6% slower: a value only later arms read was answered again, and claim placement took and gave up a claim around it. Answering only what is read after the run, the tally probes run 2% fewer instructions than unmerged.
- **A byte loaded was extended twice.** 22 of the workload's 26 byte loads are converted to `0..255` straight after, and the conversion did not know the load had zero-extended: what a conversion takes nothing off now counts how wide the load that read its value was.
- **Together** inflate runs 235.5 million instructions where it ran 243.8, 12.5 times C, and PNG decoding 3.18 billion where it ran 3.39, 6.3% fewer; every image is smaller. Against the trunk the score is 0.9957, 503 cells better and 10 worse, none by more than 0.4%.
- **A buffer's header was added in a register before every read.** Legalization writes an element's address as its start plus the header plus the index; x86 wrote the header as `push`, `pop` and `add`. Simplify now reads `x + (c + y)` at `k` as `x + y` at `k + c`, for a small `c` and a sum nothing else reads: inflate runs 226 million instructions where it ran 235.5, PNG decoding 3.6% fewer, score 0.9895 against the trunk.
- **A bounded sum was tied to its operand.** A sum of a position below 2^47 and a count is worked out across the whole register like a word's, but only an exactly 64-bit sum answered anywhere, so `a + count` wrote over `a`, copied away and back each round of `matching`: 0.8% fewer on inflate once every sum wider than 32 bits answers anywhere.
- **A condition set into a register was tested again.** Where the branch's condition was set by the instruction just before and nothing jumps in between, the branch now jumps on the flags: 0.65% fewer on inflate.
- **A small natural divided by eight was a hardware division.** The divisor reaches simplify as a constant held at exactly itself converted to a word, and simplify looked only at constants and its own folds. Reading a number through a conversion to a class it fits makes it a shift, and lets every rule asking for a number see such constants: inflate 2.6% fewer instructions, PNG 1.9%, score 0.9847 with 737 cells better and 2 worse. Inflate's cycles rose 1.3% nonetheless: the hot `matching` loop moved 30 bytes earlier, and its share of the cycles moved with it, which is placement rather than work.
- **Together** inflate runs 217 million instructions, 11.5 times C, and PNG decoding 3.0 billion.
- **A natural's tag tested where it stands, not copied first.** 306 of the workload's 317 `and $1` masks follow a copy of the register they mask. Untying a one-bit conversion and writing a branch-only one as `test $1, r8` turned every such `and` into a `test` but left the copies where they were, and added copies where a masked result landed elsewhere: inflate 1.8% more instructions, PNG 0.7% more, images larger, so it was not kept. The copy is the allocator moving the value out of the way of the edge after, not the tie.

## What a loop never reads

- **A loop's head keeps in registers what its entry held.** A block starts where its first allocated predecessor left its values, so at a loop's head every value its entry held stays in a register, read inside the loop or not. The loop's own values then put those out somewhere inside, and the edge going back brings each back every time round: the inflate workload's `matching` loop reloaded five such values from the stack each round.
- **Putting all of them in memory at the head** took inflate 2.9% fewer instructions and 10% fewer cycles (39.6 million against 43.9), but made the sort probes 8% slower: a value waiting in memory is written where it is made, so an inner loop's head waiting out its outer loop's values wrote them every time round the outer loop. Limiting it to as many as the loop holds more of than there are registers left the sort probes 7.5% slower.
- **Only what an edge going back actually brings back**, found by allocating, then allocating again with that waiting at the head until nothing more turns up, kept most of inflate's gain (204.6 million instructions, 40.7 million cycles) and made the sort probes 1.2% faster. A value moved to another register counts, not only one put in memory; counting memory alone found almost nothing. Allocating the whole program again cost up to 42% more compiling, past the guard's 25%: rejected. Allocating again only the functions with more to put out ran wrong, a state from the first pass outliving it somewhere, and it made PNG decoding's image 10% larger whichever way. The approach wants allocation per function, so that going again is local and cheap, and a value waiting in memory written where it is first needed there rather than wherever it is made. Kept in the scratchpad, not in git.
- **What the production allocators do differently.** Ours walks each function's blocks once, each starting where an allocated predecessor left its values, puts in memory the value read furthest ahead, and wrote every value that ever waits in memory where it is made. regalloc2's Ion and regalloc3 instead split each value's live range over the whole function, weighting uses by how often their blocks run, so a value a loop never reads is split out at the loop's boundary as a matter of course; and they write a value where it goes into memory, a slot holding it for its whole life so nothing is written twice. Allocating twice to look ahead cost a 42% rise in compiling; the two ideas taken on their own terms cost about nothing.
- **Written where it goes into memory.** A value going into memory in one place, in no deeper a loop than where it is made, is now written there, and a move into memory every path to it has written already writes nothing, settled per function over the blocks. Alone it took inflate 0.8% fewer instructions and made every image a little smaller; worked out across the whole program at once it compiled 7.5% more, a function's own values being all that matter.
- **A loop's head as Braun and Hack start it.** As many values the loop never reads as the loop holds more live values than there are registers now wait in memory from the head, less what already waits there: counting what was already in memory is what stopped the sort probes running 7% slower, their four-block loops holding nineteen values with six already out. With both, inflate runs 203 million instructions and 11% fewer cycles, PNG decoding 1% fewer, fasta 7% fewer, the match probe 5%; score 0.9987, compiling pidigits 4.9% more, most of it the extra moves planned and jumps settled.

## Images built from known lists

- **A known list of naturals was built a push at a time.** The gzip workload's image was 519 KB against zlib's 14 KB, and three functions were 60% of its machine IR: `inflate_from` and two copies of the deflater's `block`. `block` grew from 511 lines of Barb to 2,846 machine instructions, 720 of them pushes building lists of small naturals known before the program runs, such as `List.repeat(0, n)`, and legalization made each push a reserve call, a store and their bookkeeping, 16,438 instructions in all. Lowering laid such a list out as data only where an element was held as a bounded number; a byte read as a natural is held in place on every target, as `laid_out` already allowed, so it is data too. The gzip image is 259 KB, png's 401 KB where it was 669, and compiling gzip takes 5.3 billion instructions where it took 9.3; score 0.9752 against the trunk, size 4.4% and `--small` 5.6% smaller across the suite.
- **A large definition copied to fold one known bit.** Measured by `--symbols`, the gzip image's text was 229 KB, and two copies of the deflater's `block`, 18.5 KB each, differed in two constants: the specializer copied it for whether a block is the last, read in one place, because each copy decided one match of its 48. A copy is now kept only where it is small, at most 256 in what copying allocates, or costs at most three quarters of what it was copied from. At 64, fasta's copies for its known strings, 90 apiece, were refused too, and its 6502 memory rose 19%; at 256 they stay. gzip's image is 237 KB where it was 259 and png's 377 KB where it was 401, for gzip 1.5% more instructions, the price of reading the bit; score 0.9972 against the trunk.

## The words' counts of their bits

- **Recognized by shape, answered where a target has the instruction.** Barb reads a definition taking a word as arithmetic modulo its width and recognizes Hacker's Delight's count of ones, the count of ones of `!w & (w - 1)`, and the width less the count of ones of the word smeared down from its highest bit. x86-64 answers the counts of zeros with `bsf` and `bsr` and a jump for nought; wasm32 answers all three with `i32.popcnt`, `i32.ctz` and `i32.clz`, a `U64` a half at a time; the 6502 walks every body.
- **What one call costs**, a `U64` in a loop of a million on x86-64 and a hundred thousand on wasm32, beside the loop alone (49 instructions, 225 fuel a round): `trailing_zeros` 26 instructions to 0 over the loop, 360 fuel to 10; `leading_zeros` 43 to 1, 426 to 9; `count_ones` 21 to 21 on x86-64, where it stays its arithmetic at the baseline as rustc's does, and 273 to 4 on wasm32. The `bits` probe runs 74 instructions a round where it ran 97 on x86-64, 23.7% fewer, and 251 fuel where it ran 857 on wasm32, 70.7% less.
- **Neutral against the trunk** at 0.99942: the probe is one cell in the probes' group, and only perft among the suite counts bits, 0.75% fewer instructions on x86-64 and 2.2% less fuel on wasm32. Compiling the probe for wasm32 took 6% more instructions, from the code it now writes rather than from reading for the counts, which a dozen readings of a few hundred terms each come to.
- **`popcnt` would answer `count_ones`** on x86-64 in one instruction where the baseline takes 21; the switch for it is in the target and off, as the images run on every x86-64.
