/* Count the paths of legal chess moves to a depth from two positions, the
   start and Kiwipete: perft, the count that checks a move generator. The
   board is 64 squares, a1 first. Every move the pieces could make is
   played on a copy of the position, and counted where it leaves the
   mover's king unattacked. */
#include <stdio.h>
#include <stdlib.h>

enum { EMPTY, PAWN, KNIGHT, BISHOP, ROOK, QUEEN, KING };
/* A piece is its role, with BLACK added for Black's. */
enum { WHITE = 0, BLACK = 8 };
enum { WHITE_KING_SIDE = 1, WHITE_QUEEN_SIDE = 2, BLACK_KING_SIDE = 4, BLACK_QUEEN_SIDE = 8 };
enum { NORMAL, DOUBLE_PUSH, EN_PASSANT, CASTLE };

#define MAX_DEPTH 16
#define MAX_MOVES 256

struct position {
    unsigned char board[64];
    unsigned char turn;
    unsigned char castling;
    int en_passant; /* the square, or -1 */
};

struct move {
    unsigned char from, to, promotion, kind;
};

static const int straight[4][2] = {{0, 1}, {1, 0}, {0, -1}, {-1, 0}};
static const int diagonal[4][2] = {{1, 1}, {1, -1}, {-1, -1}, {-1, 1}};
static const int knight[8][2] = {{-1, 2}, {1, 2}, {2, 1}, {2, -1}, {1, -2}, {-1, -2}, {-2, -1}, {-2, 1}};
static const int king[8][2] = {{0, 1}, {1, 0}, {0, -1}, {-1, 0}, {1, 1}, {1, -1}, {-1, -1}, {-1, 1}};
static const unsigned char promotions[4] = {QUEEN, ROOK, BISHOP, KNIGHT};

static struct move moves[MAX_DEPTH + 1][MAX_MOVES];

/* The square `files` and `ranks` away from `square`, or -1 off the board. */
static int step(int square, int files, int ranks) {
    int file = square % 8 + files, rank = square / 8 + ranks;
    if (file < 0 || file > 7 || rank < 0 || rank > 7) return -1;
    return rank * 8 + file;
}

static int color_of(int piece) { return piece & BLACK; }
static int role_of(int piece) { return piece & 7; }

/* The first piece met from `square` going `files` and `ranks` at a time. */
static int first_along(const struct position *p, int square, int files, int ranks) {
    for (;;) {
        square = step(square, files, ranks);
        if (square < 0) return EMPTY;
        if (p->board[square] != EMPTY) return p->board[square];
    }
}

/* Whether a piece of `by` attacks `square`. */
static int attacked(const struct position *p, int square, int by) {
    int i, from, piece, ahead = by == WHITE ? 1 : -1;
    for (i = 0; i < 4; i++) {
        piece = first_along(p, square, straight[i][0], straight[i][1]);
        if (piece == (by | ROOK) || piece == (by | QUEEN)) return 1;
        piece = first_along(p, square, diagonal[i][0], diagonal[i][1]);
        if (piece == (by | BISHOP) || piece == (by | QUEEN)) return 1;
    }
    for (i = 0; i < 8; i++) {
        from = step(square, knight[i][0], knight[i][1]);
        if (from >= 0 && p->board[from] == (by | KNIGHT)) return 1;
        from = step(square, king[i][0], king[i][1]);
        if (from >= 0 && p->board[from] == (by | KING)) return 1;
    }
    for (i = -1; i <= 1; i += 2) {
        from = step(square, i, -ahead);
        if (from >= 0 && p->board[from] == (by | PAWN)) return 1;
    }
    return 0;
}

static int king_square(const struct position *p, int color) {
    int square;
    for (square = 0; square < 64; square++)
        if (p->board[square] == (color | KING)) return square;
    return 0;
}

static int add(struct move *list, int count, int from, int to, int promotion, int kind) {
    list[count].from = (unsigned char)from;
    list[count].to = (unsigned char)to;
    list[count].promotion = (unsigned char)promotion;
    list[count].kind = (unsigned char)kind;
    return count + 1;
}

/* A pawn's move to `to`, as each role it may become on the last rank. */
static int add_pawn(struct move *list, int count, int from, int to) {
    int i;
    if (to / 8 == 0 || to / 8 == 7) {
        for (i = 0; i < 4; i++) count = add(list, count, from, to, promotions[i], NORMAL);
        return count;
    }
    return add(list, count, from, to, EMPTY, NORMAL);
}

/* The squares of a castle: the king's from and to, the rook's from and to. */
static void castle_squares(int color, int king_side, int *squares) {
    int base = color == WHITE ? 0 : 56;
    squares[0] = base + 4;
    squares[1] = base + (king_side ? 6 : 2);
    squares[2] = base + (king_side ? 7 : 0);
    squares[3] = base + (king_side ? 5 : 3);
}

/* Every move the side to move's pieces could make, into `list`. */
static int candidates(const struct position *p, struct move *list) {
    int count = 0, from, to, i, piece, role, us = p->turn, them = us ^ BLACK;
    int ahead = us == WHITE ? 1 : -1, start = us == WHITE ? 1 : 6;
    for (from = 0; from < 64; from++) {
        piece = p->board[from];
        if (piece == EMPTY || color_of(piece) != us) continue;
        role = role_of(piece);
        if (role == PAWN) {
            to = step(from, 0, ahead);
            if (to >= 0 && p->board[to] == EMPTY) {
                count = add_pawn(list, count, from, to);
                to = step(to, 0, ahead);
                if (from / 8 == start && to >= 0 && p->board[to] == EMPTY)
                    count = add(list, count, from, to, EMPTY, DOUBLE_PUSH);
            }
            for (i = -1; i <= 1; i += 2) {
                to = step(from, i, ahead);
                if (to < 0) continue;
                if (p->board[to] != EMPTY && color_of(p->board[to]) == them)
                    count = add_pawn(list, count, from, to);
                else if (p->board[to] == EMPTY && to == p->en_passant)
                    count = add(list, count, from, to, EMPTY, EN_PASSANT);
            }
        } else if (role == KNIGHT || role == KING) {
            for (i = 0; i < 8; i++) {
                to = role == KNIGHT ? step(from, knight[i][0], knight[i][1]) : step(from, king[i][0], king[i][1]);
                if (to >= 0 && (p->board[to] == EMPTY || color_of(p->board[to]) == them))
                    count = add(list, count, from, to, EMPTY, NORMAL);
            }
        } else {
            for (i = 0; i < 8; i++) {
                const int *way = i < 4 ? straight[i] : diagonal[i - 4];
                if ((role == ROOK && i >= 4) || (role == BISHOP && i < 4)) continue;
                for (to = step(from, way[0], way[1]); to >= 0; to = step(to, way[0], way[1])) {
                    if (p->board[to] == EMPTY) {
                        count = add(list, count, from, to, EMPTY, NORMAL);
                    } else {
                        if (color_of(p->board[to]) == them) count = add(list, count, from, to, EMPTY, NORMAL);
                        break;
                    }
                }
            }
        }
    }
    for (i = 0; i < 2; i++) {
        int squares[4], right, low, high, square, clear = 1;
        right = us == WHITE ? (i == 0 ? WHITE_KING_SIDE : WHITE_QUEEN_SIDE)
                            : (i == 0 ? BLACK_KING_SIDE : BLACK_QUEEN_SIDE);
        if (!(p->castling & right)) continue;
        castle_squares(us, i == 0, squares);
        low = squares[0] < squares[2] ? squares[0] : squares[2];
        high = squares[0] < squares[2] ? squares[2] : squares[0];
        for (square = low + 1; square < high; square++)
            if (p->board[square] != EMPTY) clear = 0;
        low = squares[0] < squares[1] ? squares[0] : squares[1];
        high = squares[0] < squares[1] ? squares[1] : squares[0];
        for (square = low; square <= high; square++)
            if (attacked(p, square, them)) clear = 0;
        if (clear) count = add(list, count, squares[0], squares[1], EMPTY, CASTLE);
    }
    return count;
}

/* The right lost by a rook moving from, or taken on, `square`. */
static int corner_right(int square) {
    switch (square) {
    case 0: return WHITE_QUEEN_SIDE;
    case 7: return WHITE_KING_SIDE;
    case 56: return BLACK_QUEEN_SIDE;
    case 63: return BLACK_KING_SIDE;
    default: return 0;
    }
}

static void play(struct position *p, const struct move *m) {
    int us = p->turn, piece = p->board[m->from], squares[4];
    p->board[m->from] = EMPTY;
    p->board[m->to] = (unsigned char)(m->promotion != EMPTY ? (us | m->promotion) : piece);
    if (m->kind == EN_PASSANT) p->board[(m->from / 8) * 8 + m->to % 8] = EMPTY;
    if (m->kind == CASTLE) {
        castle_squares(us, m->to % 8 == 6, squares);
        p->board[squares[2]] = EMPTY;
        p->board[squares[3]] = (unsigned char)(us | ROOK);
    }
    if (role_of(piece) == KING)
        p->castling &= us == WHITE ? ~(WHITE_KING_SIDE | WHITE_QUEEN_SIDE) : ~(BLACK_KING_SIDE | BLACK_QUEEN_SIDE);
    p->castling &= ~(corner_right(m->from) | corner_right(m->to));
    p->en_passant = m->kind == DOUBLE_PUSH ? (m->from + m->to) / 2 : -1;
    p->turn = (unsigned char)(us ^ BLACK);
}

static unsigned long perft(const struct position *p, int depth) {
    struct move *list = moves[depth];
    struct position next;
    int count, i;
    unsigned long total = 0;
    if (depth == 0) return 1;
    count = candidates(p, list);
    for (i = 0; i < count; i++) {
        next = *p;
        play(&next, &list[i]);
        if (attacked(&next, king_square(&next, p->turn), next.turn)) continue;
        total += depth == 1 ? 1 : perft(&next, depth - 1);
    }
    return total;
}

/* The position the first four fields of a FEN write. */
static void read_fen(const char *fen, struct position *p) {
    int square = 56, piece;
    const char *roles = " pnbrqk", *found;
    for (square = 0; square < 64; square++) p->board[square] = EMPTY;
    square = 56;
    for (; *fen != ' '; fen++) {
        if (*fen == '/') {
            square -= 16;
        } else if (*fen >= '1' && *fen <= '8') {
            square += *fen - '0';
        } else {
            piece = *fen >= 'a' ? *fen : *fen - 'A' + 'a';
            for (found = roles; *found != piece; found++) {}
            p->board[square++] = (unsigned char)((found - roles) | (*fen >= 'a' ? BLACK : WHITE));
        }
    }
    p->turn = *++fen == 'w' ? WHITE : BLACK;
    fen += 2;
    p->castling = 0;
    for (; *fen != ' '; fen++) {
        if (*fen == 'K') p->castling |= WHITE_KING_SIDE;
        if (*fen == 'Q') p->castling |= WHITE_QUEEN_SIDE;
        if (*fen == 'k') p->castling |= BLACK_KING_SIDE;
        if (*fen == 'q') p->castling |= BLACK_QUEEN_SIDE;
    }
    fen++;
    p->en_passant = *fen == '-' ? -1 : (fen[1] - '1') * 8 + fen[0] - 'a';
}

int main(int argc, char **argv) {
    int depth = argc > 1 ? atoi(argv[1]) : 2;
    struct position start, kiwipete;
    if (depth < 0 || depth > MAX_DEPTH) return 1;
    read_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1", &start);
    read_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", &kiwipete);
    printf("start %lu\n", perft(&start, depth));
    printf("kiwipete %lu\n", perft(&kiwipete, depth));
    return 0;
}
