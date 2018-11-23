extern crate peg;

fn main() {
    peg::cargo_build("src/css_parser/css_grammar.rustpeg");
}
