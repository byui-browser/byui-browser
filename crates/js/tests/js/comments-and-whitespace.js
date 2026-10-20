/* Comments and unusual spacing should not change evaluation. */
let answer = 40; // trailing comment
let increment = 1;

answer = answer + /* inline comment */ increment;
// A second expression statement exercises comments between statements.
answer = answer + 1;
answer;
