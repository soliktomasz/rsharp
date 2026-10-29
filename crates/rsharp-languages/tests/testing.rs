use rsharp_languages::{Grammar, parse, testing};
use rsharp_text::{Document, Rope, Selection, insert};

fn read(source: &str) -> testing::FileTests {
    let text = Rope::from_str(source);
    testing::read(&parse(Grammar::CSharp, &text, None), &text)
}

#[test]
fn framework_attributes_are_recognized_without_going_into_method_bodies() {
    let attributes = [
        "Fact",
        "Theory",
        "AvaloniaFact",
        "AvaloniaTheory",
        "Test",
        "TestCase(1)",
        "TestCaseSource(nameof(Data))",
        "TestMethod",
        "DataTestMethod",
        "global::Xunit.FactAttribute",
        "NUnit.Framework.TestAttribute",
    ];
    for attribute in attributes {
        let source = format!(
            "namespace App.Tests;\nclass Cases {{ [{attribute}] void Works() {{ void Local() {{ }} }} void Helper() {{ }} }}"
        );
        let found = read(&source);
        assert_eq!(found.tests.len(), 1, "[{attribute}] did not produce one test");
        assert_eq!(found.tests[0].type_name, "App.Tests.Cases");
        assert_eq!(found.tests[0].method, "Works");
        assert_eq!(found.tests[0].line, 2);
        assert_eq!(found.types, [("App.Tests.Cases".into(), 2)]);
        assert_eq!(found.methods, ["Helper".into(), "Works".into()].into());
    }
    assert!(read("class Cases { [NotATest] void Helper() {} }").tests.is_empty());
}

#[test]
fn nesting_records_and_abstract_or_generic_owners_keep_their_identity() {
    let found = read(
        "namespace One { namespace Two { abstract class Outer { class Inner { [Fact] void A() {} } } record Cases { [Test] void B() {} } class Generic<T> { class Inner { [Fact] void C() {} } } struct Value { [TestMethod] void D() {} } } }",
    );
    let identities: Vec<_> = found.tests.iter().map(|t| (t.type_name.as_str(), t.abstract_type)).collect();
    assert_eq!(
        identities,
        [
            ("One.Two.Outer+Inner", false),
            ("One.Two.Cases", false),
            ("One.Two.Generic+Inner", true),
            ("One.Two.Value", false)
        ]
    );
}

#[test]
fn nested_concrete_classes_are_runnable_inside_abstract_but_not_open_generic_owners() {
    let found = read(
        "abstract class Outer { [Fact] void BaseTest() {} class Inner { [Fact] void NestedTest() {} } } class Generic<T> { class Inner { [Fact] void GenericTest() {} } }",
    );
    let inherited: Vec<_> = found.tests.iter().map(|t| (t.method.as_str(), t.abstract_type)).collect();
    assert_eq!(
        inherited,
        [("BaseTest", true), ("NestedTest", false), ("GenericTest", true)],
        "a containing abstract class hid a concrete nested test, or an open generic test became runnable"
    );
}

#[test]
fn class_traits_precede_method_traits_and_only_literal_arguments_count() {
    let found = read(
        r###"[Trait("Area", "Files"), Category(@"slow"), TestCategory("unit")]
class Cases {
    [Fact, Trait("Name", "caf\u00e9\n"), Category("fast"), Trait(Name = "ignored", Value = "also"), Trait("constant", Value)]
    void Works() {}
}"###,
    );
    assert_eq!(
        found.tests[0].traits,
        [
            ("Area".into(), "Files".into()),
            ("Category".into(), "slow".into()),
            ("TestCategory".into(), "unit".into()),
            ("Name".into(), "café\n".into()),
            ("Category".into(), "fast".into())
        ]
    );
}

#[test]
fn raw_trait_strings_keep_their_literal_values() {
    let found = read(
        r####"class Cases {
    [Fact, Trait("""Area""", """
        first
        second
        """)]
    void Works() {}
}"####,
    );
    assert_eq!(found.tests[0].traits, [("Area".into(), "first\nsecond".into())]);
}

#[test]
fn preprocessor_wrappers_do_not_hide_test_methods() {
    let found = read("class Cases {\n#if DEBUG\n[Fact] void Works() {}\n#endif\n}");
    assert_eq!(found.tests.len(), 1, "a preprocessor wrapper hid its test declaration");
    assert!(found.methods.contains("Works"));
}

#[test]
fn caret_context_and_incremental_edits_use_unicode_byte_positions() {
    let source =
        "namespace Próby;\nclass Żółć\n{\n    [Fact]\n    void Działa()\n    {\n        var znak = \"😀\";\n    }\n}\n";
    let mut document = Document::new(source);
    let mut tree = parse(Grammar::CSharp, document.text(), None);
    assert_eq!(testing::at(&tree, document.text(), 7), Some(("Próby.Żółć".into(), Some("Działa".into()))));
    assert_eq!(testing::at(&tree, document.text(), 3), Some(("Próby.Żółć".into(), None)));
    assert_eq!(testing::at(&tree, document.text(), 0), None);
    assert_eq!(testing::at(&tree, document.text(), 100), None);
    document.set_selections(vec![Selection::caret(source.find("    [Fact]").unwrap())]);
    for change in document.apply(insert(document.text(), document.selections(), "    // 😀\n")) {
        rsharp_languages::edit(&mut tree, &change);
    }
    tree = parse(Grammar::CSharp, document.text(), Some(&tree));
    let found = testing::read(&tree, document.text());
    assert_eq!(found.tests[0].line, 6, "a Unicode edit moved the test to the wrong source line");
    assert_eq!(found, testing::read(&parse(Grammar::CSharp, document.text(), None), document.text()));
}

#[test]
fn blank_caret_lines_do_not_select_the_next_lines_declaration() {
    for blank in ["\n", " \t\n", "\r\n", " \t\r\n"] {
        let text = Rope::from_str(&format!("class Cases\n{{\n{blank}[Fact] void Works() {{}}\n}}\n"));
        let tree = parse(Grammar::CSharp, &text, None);
        assert_eq!(
            testing::at(&tree, &text, 3),
            Some(("Cases".into(), None)),
            "blank line {blank:?} selected the following test method"
        );
        assert_eq!(testing::at(&tree, &text, 4), Some(("Cases".into(), Some("Works".into()))));
        let text = Rope::from_str(&format!("{blank}class Cases {{}}"));
        assert_eq!(
            testing::at(&parse(Grammar::CSharp, &text, None), &text, 1),
            None,
            "a blank line outside a type selected the following type"
        );
    }
}
