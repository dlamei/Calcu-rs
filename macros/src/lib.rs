use proc_macro2::{TokenStream, Span};
use quote::{quote, ToTokens};
use syn::{parse::{discouraged::Speculative, Parse, ParseStream}, punctuated as punc, Token};
use std::fmt::Write;

#[derive(PartialEq, Eq, Clone, Copy, Hash, PartialOrd, Ord)]
enum OpKind {
    Add, Sub,
    Mul, Div,
    Pow,
}

impl std::fmt::Debug for OpKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let str = match self {
            OpKind::Add => "Add",
            OpKind::Sub => "Sub",
            OpKind::Mul => "Mul",
            OpKind::Div => "Div",
            OpKind::Pow => "Pow",
        };
        write!(f, "{}", str)
    }
}

#[derive(Debug, Clone, Copy)]
struct Op {
    kind: OpKind,
    span: Span,
}


impl OpKind {
    fn precedence(&self) -> i32 {
        match self {
            OpKind::Add | OpKind::Sub => 1,
            OpKind::Mul | OpKind::Div => 2,
            OpKind::Pow => 3,
        }
    }
}

impl Op {
    fn precedence(&self) -> i32 {
        self.kind.precedence()
    }
}

impl Parse for Op {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let (kind, span) =
            if let Ok(op) = input.parse::<Token![+]>() {
                (OpKind::Add, op.span)
            } else if let Ok(op) = input.parse::<Token![-]>() {
                (OpKind::Sub, op.span)
            } else if let Ok(op) = input.parse::<Token![*]>() {
                (OpKind::Mul, op.span)
            } else if let Ok(op) = input.parse::<Token![/]>() {
                (OpKind::Div, op.span)
            } else if let Ok(op) = input.parse::<Token![^]>() {
                (OpKind::Pow, op.span)
            } else {
                return Err(syn::parse::Error::new(input.span(), "expected operator { +, -, *, / }"));
            };
        Ok(Self {kind, span})
    }
}

#[derive(Debug, PartialEq, Clone, PartialOrd)]
enum Expr {
    Num(i32),
    Float(f32),
    Symbol(String),
    Binary(OpKind, Box<Expr>, Box<Expr>),
    Infinity{sign: i8},
    Undef,
    PlaceHolder(String),
}

impl Expr {
    fn parse_operand(s: ParseStream) -> syn::Result<Expr> {
        if let Ok(id) = syn::Ident::parse(s) {
            let id = id.to_string();
            if id == "oo" {
                Ok(Expr::Infinity { sign: 1 })
            } else if id == "undef" {
                Ok(Expr::Undef)
            } else {
                Ok(Expr::Symbol(id.to_string()))
            }

        } else if let Ok(i) = syn::LitInt::parse(s) {
            let val: i32 = i.base10_parse().unwrap();
            Ok(Expr::Num(val))

        } else if let Ok(f) = syn::LitFloat::parse(s) {
            let val: f32 = f.base10_parse().unwrap();
            Ok(Expr::Float(val))

        } else if s.peek(syn::token::Paren) {
            let content;
            syn::parenthesized!(content in s);
            Expr::parse(&content)
        } else {
            Err(syn::parse::Error::new(s.span(), "bad expression"))
        }
    }

    fn parse_unary_expr(s: ParseStream) -> syn::Result<Expr> {
        if let Ok(op) = Op::parse(s) {
            match op.kind {
                OpKind::Sub => {
                    let operand = Self::parse_operand(s)?;
                    Ok(Expr::Binary(OpKind::Mul, Expr::Num(-1).into(), operand.into()))
                }
                _ => Err(syn::parse::Error::new(op.span, "expected unary operator"))
            }
        }  else if let Ok(_) = s.parse::<Token![?]>() {
            let mut id = "?".to_string();
            id.push_str(&syn::Ident::parse(s)?.to_string());
            Ok(Expr::PlaceHolder(id.to_string()))
        } else {
            Self::parse_operand(s)
        }
    }
    fn parse_bin_expr(s: ParseStream, prec_in: i32) -> syn::Result<Expr> {
        let mut expr = Self::parse_unary_expr(s)?;
        loop
        {
            if s.is_empty() {
                break;
            }

            if s.peek(Token![->]) || (s.peek(Token![<]) && s.peek2(Token![->])) || s.peek(Token![;]) {
                break;
            }

            let ahead = s.fork();
            let op = match Op::parse(&ahead) {
                Ok(op) if op.precedence() < prec_in => break,
                Ok(op) => op,
                Err(_) => break,
            };

            s.advance_to(&ahead);

            let rhs = Expr::parse_bin_expr(s, op.precedence() + 1)?;
            expr = Expr::Binary(op.kind, expr.into(), rhs.into());
        }

        Ok(expr)
    }

    fn eval_op(op: OpKind, lhs: TokenStream, rhs: TokenStream) -> TokenStream {
        match op {
            OpKind::Add => quote!((#lhs + #rhs)),
            OpKind::Sub => quote!((#lhs - #rhs)),
            OpKind::Mul => quote!((#lhs * #rhs)),
            OpKind::Div => quote!((#lhs / #rhs)),
            OpKind::Pow => quote!((#lhs.pow(#rhs))),
        }
    }

    fn quote(&self) -> TokenStream {
        match self {
            Expr::Num(v) =>
                quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Rational::from(#v))),
            Expr::Float(v) =>
                quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Float::from(#v))),
            Expr::Symbol(s) =>
                quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Symbol::new(#s))),
                Expr::Binary(op, l, r) => {
                    let lhs = l.quote();
                    let rhs = r.quote();
                    Self::eval_op(*op, lhs, rhs)
                }
            Expr::Infinity { sign } => {
                if sign.is_negative() {
                    quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Infinity::neg()))
                } else {
                    quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Infinity::pos()))
                }
            }
            Expr::Undef => {
                quote!(::calcu_rs::prelude::Expr::Undefined)
            }
            Expr::PlaceHolder(s) => {
                quote!(::calcu_rs::prelude::Expr::PlaceHolder(#s))
            }
        }
    }

}

impl syn::parse::Parse for Expr {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        Expr::parse_bin_expr(input, 0 + 1)
    }
}

//fn eval_expr(expr: &Expr) -> TokenStream {
//    match expr {
//        Expr::Num(v) =>
//            quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Rational::from(#v))),
//        Expr::Float(v) =>
//            quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Float::from(#v))),
//        Expr::Symbol(s) =>
//            quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Symbol::new(#s))),
//        Expr::Binary(op, l, r) => {
//            let lhs = eval_expr(l);
//            let rhs = eval_expr(r);
//            eval_op(*op, lhs, rhs)
//        }
//        Expr::Infinity { sign } => {
//            if sign.is_negative() {
//                quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Infinity::neg()))
//            } else {
//                quote!(::calcu_rs::prelude::Expr::from(::calcu_rs::prelude::Infinity::pos()))
//            }
//        }
//        Expr::Undef => {
//            quote!(::calcu_rs::prelude::Expr::Undefined)
//        }
//        Expr::PlaceHolder(s) => {
//            quote!(::calcu_rs::prelude::Expr::PlaceHolder(#s))
//        }
//    }
//}

#[proc_macro]
pub fn calc(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    syn::parse_macro_input!(input as Expr).quote().into()
}

#[derive(Debug, Clone)]
struct RewriteRule {
    name: String,
    lhs: Expr,
    rhs: Expr,
    cond: Option<syn::Expr>,
    bidir: bool,
}

impl RewriteRule {

    fn quote_lhs_to_rhs(name: &String, lhs: &Expr, rhs: &Expr, cond: &Option<syn::Expr>, dbg: bool) -> TokenStream {
        let lhs = lhs.quote();
        let rhs = rhs.quote();

        let mut debug = TokenStream::new();
        if dbg {
            let cond_str =
                match cond {
                    Some(cond) => {
                        let mut str = " if ".to_string();
                        write!(str, "{},", cond.clone().to_token_stream().to_string()).unwrap();
                        str
                    },
                    None => ",".into(),
                };

            debug = quote!(
                println!("  {}: {} => {}{}", #name, __searcher, __applier, #cond_str);
                )
        }

        let mut cond_applier = TokenStream::new();

        if let Some(cond) = cond {
            cond_applier = quote!(
                let __applier = ::egg::ConditionalApplier {
                    condition: #cond,
                    applier: __applier,
                };
                )
        }

        quote!({
            let __searcher = ::egg::Pattern::from(&#lhs);
            let __applier  = ::egg::Pattern::from(&#rhs);
            #debug
            #cond_applier
            ::egg::Rewrite::new(#name.to_string(), __searcher, __applier).unwrap()
        })
    }

    fn quote_debug(&self, dbg: bool) -> TokenStream {
        if self.bidir {
            let n1 = self.name.clone();
            let mut n2 = self.name.clone();
            n2.push_str(" REV");
            let r1 = Self::quote_lhs_to_rhs(&n1, &self.lhs, &self.rhs, &self.cond, dbg);
            let r2 = Self::quote_lhs_to_rhs(&n2, &self.rhs, &self.lhs, &self.cond, dbg);
            quote!(#r1, #r2)
        } else {
            Self::quote_lhs_to_rhs(&self.name, &self.lhs, &self.rhs, &self.cond, dbg)
        }
    }
}

impl Parse for RewriteRule {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut name = syn::Ident::parse(input)?.to_string();

        loop {
            if let Ok(n) = syn::Ident::parse(input) {
                name.push_str(" ");
                name.push_str(&n.to_string());
            } else if let Ok(n) = syn::Lit::parse(input) {
                name.push_str(" ");
                name.push_str(&n.to_token_stream().to_string());
            } else {
                break;
            }
        }

        let _ = input.parse::<Token![:]>()?;

        let lhs = Expr::parse(input)?;

        let bidir = 
            if input.peek(Token![->]) {
                let _ = input.parse::<Token![->]>()?;    
                false
            } else if input.peek(Token![<]) && input.peek2(Token![->]) {
                let _ = input.parse::<Token![<]>()?;    
                let _ = input.parse::<Token![->]>()?;    
                true
            } else {
                return Err(syn::parse::Error::new(input.span(), "expected -> or <->"));
            };

        let rhs = Expr::parse(input)?;

        let cond =
            if let Ok(_) = input.parse::<Token![if]>() {
                Some(syn::Expr::parse(input)?)
            } else {
                None
            };

        Ok(RewriteRule { name, lhs, rhs, cond, bidir })
    }
}

#[derive(Debug, Clone)]
struct RuleSet {
    gen_name: syn::Ident,
    rules: Vec<RewriteRule>,
    debug: bool,
}

impl RuleSet {
    fn quote(&self) -> TokenStream {
        let gen_name = &self.gen_name;

        let mut n: usize = 0;
        for r in &self.rules {
            n += 
                if r.bidir {
                    2
                } else {
                    1
                };
        }

        let mut rules = TokenStream::new();
        for r in &self.rules {
            let r = r.quote_debug(self.debug);
            rules.extend(quote!(#r,))
        }

        let mut debug = TokenStream::new();
        if self.debug {
            let name = gen_name.to_string();
            debug = quote!(println!("{}:", #name););
        }

        quote!(
            pub fn #gen_name() -> [::egg::Rewrite<Self, <Self as GraphExpression>::Analyser>; #n] {
                #debug
                [ #rules ]
            }
            )
    }
}

impl Parse for RuleSet {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut gen_name = syn::Ident::parse(input)?;
        let mut debug = false;

        if gen_name == "debug" {
            debug = true;
            gen_name = syn::Ident::parse(input)?;
        }

        let _ = input.parse::<Token![:]>();
        let rules: Vec<_> = punc::Punctuated::<RewriteRule, syn::Token![,]>::parse_terminated(&input)?.
            into_iter().collect();

        Ok(RuleSet { gen_name, rules, debug })
    }
}

fn op_to_node(op: OpKind) -> TokenStream {
    match op {
        OpKind::Add => quote!(Node::Add),
        OpKind::Sub => todo!(),
        OpKind::Mul => quote!(Node::Mul),
        OpKind::Div => todo!(),
        OpKind::Pow => quote!(Node::Pow),
    }
}

fn to_node_rec(e: Expr) -> TokenStream {
    //let var = syn::Ident::new(node_name, Span::call_site());
    match e {
        Expr::Num(n) => quote!(graph.add_raw(Node::Rational(Rational::from(#n)))),
        Expr::Symbol(s) => quote!(graph.add_raw(Node::Symbol(#s.into()))),
            Expr::Binary(op, lhs, rhs) => {
                let lhs = to_node_rec(*lhs);
                let rhs = to_node_rec(*rhs);
                let op = op_to_node(op);
                quote!({
                    let lhs = #lhs;
                    let rhs = #rhs;
                    graph.add_raw(#op([lhs, rhs]))
                })
            },
            Expr::PlaceHolder(_) => todo!(),
            _ => todo!()
    }
}

fn to_node(e: Expr) -> TokenStream {
    let n = to_node_rec(e);
    quote!({
        let mut graph = ExprGraph::default();
        let _ = #n;
        graph.compact()
    })
}

#[proc_macro]
pub fn expr(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let expr = syn::parse_macro_input!(input as Expr);
    let stream = to_node(expr);
    //panic!("{}", stream);
    stream.into()
}

#[proc_macro]
pub fn define_rules(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    syn::parse_macro_input!(input as RuleSet).quote().into()
}
