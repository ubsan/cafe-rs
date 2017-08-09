use std::collections::HashMap;
use std::fmt::{self, Display};

use mir::{self, Mir};
use parse::{ItemVariant, Parser, ParserError,
            ParserErrorVariant, Spanned};
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum Category {
  Raw,
  Shared,
  Mut,
  Own,
}

// user defined types will be strings
#[derive(Clone, Debug)]
pub enum StringlyType {
  #[allow(dead_code)]
  Reference(Category, Box<StringlyType>),
  #[allow(dead_code)]
  Pointer(Category, Box<StringlyType>),
  UserDefinedType(String),
  Unit,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum BinOp {
  Plus,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum BinOpPrecedence {
  Addition
}

impl BinOp {
  pub fn precedence(self) -> BinOpPrecedence {
    match self {
      BinOp::Plus => BinOpPrecedence::Addition,
    }
  }
}

#[derive(Debug)]
pub enum ExpressionVariant {
  Nullary,
  IntLiteral(u64),
  Variable(String),
  BinOp {
    lhs: Box<Expression>,
    rhs: Box<Expression>,
    op: BinOp,
  },
  Call {
    callee: String,
    //args: ...,
  },
}
impl ExpressionVariant {
  fn mir_binop(
    op: BinOp,
    lhs: mir::Reference,
    rhs: mir::Reference,
  ) -> mir::Value {
    match op {
      BinOp::Plus => mir::Value::Add(lhs, rhs),
    }
  }
  fn to_mir<'ctx>(
    &self,
    // TODO(ubsan): this state should probably all be in a struct
    mir: &mut Mir<'ctx>,
    builder: &mut mir::FunctionBuilder<'ctx>,
    block: mir::Block,
    funcs: &HashMap<String, mir::FunctionDecl>,
    locals: &HashMap<String, mir::Reference>,
  ) -> mir::Value {
    let s32 = mir.get_type(&StringlyType::UserDefinedType(
      "s32".to_owned(),
    )).unwrap();
    match *self {
      ExpressionVariant::IntLiteral(i) => {
        mir::Value::Literal(i as i32)
      }
      ExpressionVariant::Variable(ref name) => {
        // will panic for now - should be caught in typeck
        if let Some(&loc) = locals.get(name) {
          mir::Value::Reference(loc)
        } else {
          panic!("no `{}` name found");
        }
      }
      ExpressionVariant::BinOp {
        ref lhs,
        ref rhs,
        ref op,
      } => {
        let lhs = {
          let val = lhs.to_mir(mir, builder, block, funcs, locals);
          let tmp = builder.add_anonymous_local(s32);
          builder.add_stmt(block, tmp, val);
          tmp
        };
        let rhs = {
          let val = rhs.to_mir(
            mir,
            builder,
            block,
            funcs,
            locals,
          );
          let tmp = builder.add_anonymous_local(s32);
          builder.add_stmt(block, tmp, val);
          tmp
        };
        Self::mir_binop(*op, lhs, rhs)
      }
      ExpressionVariant::Call { ref callee } => {
        if let Some(&callee) = funcs.get(callee) {
          mir::Value::Call { callee }
        } else {
          panic!("function `{}` doesn't exist", callee);
        }
      }
      ExpressionVariant::Nullary => {
        panic!("non-s32 types not yet supported")
      }
    }
  }
}
pub type Expression = Spanned<ExpressionVariant>;
#[derive(Debug)]
pub enum StatementVariant {
  Expr(Expression),
  Local {
    name: String,
    ty: StringlyType,
    initializer: Expression,
  },
}
pub type Statement = Spanned<StatementVariant>;


#[derive(Debug)]
pub struct Block_ {
  pub statements: Vec<Statement>,
  pub expr: Expression,
}
pub type Block = Spanned<Block_>;

#[derive(Debug)]
pub struct FunctionValue {
  //params: Vec<(String, StringlyType)>,
  pub ret_ty: StringlyType,
  pub blk: Block_,
}
pub type Function = Spanned<FunctionValue>;

impl Function {
  fn build_mir<'ctx>(
    &self,
    decl: mir::FunctionDecl,
    funcs: &HashMap<String, mir::FunctionDecl>,
    mir: &mut Mir<'ctx>,
  ) {
    let s32 = mir.get_type(&self.thing.ret_ty).unwrap();

    let mut locals: HashMap<String, mir::Reference> =
      HashMap::new();
    let mut builder =
      mir.get_function_builder(decl, s32);

    let block = builder.entrance();
    for stmt in &self.blk.statements {
      match **stmt {
        StatementVariant::Expr(ref e) => {
          let tmp = builder.add_anonymous_local(s32);
          let mir_val = e.to_mir(
            mir,
            &mut builder,
            block,
            funcs,
            &locals,
          );
          builder.add_stmt(block, tmp, mir_val);
        },
        StatementVariant::Local {
          ref name,
          ref ty,
          ref initializer,
        } => {
          let ty = mir.get_type(ty).unwrap();
          let ref_ = builder.add_local(name.clone(), ty);
          let init = initializer.to_mir(
            mir,
            &mut builder,
            block,
            funcs,
            &locals,
          );
          builder.add_stmt(block, ref_, init);
          locals.insert(name.clone(), ref_);
        }
      };
    }
    let mir_val = self.blk.expr.to_mir(
      mir,
      &mut builder,
      block,
      funcs,
      &locals,
    );
    let ret = mir::Reference::ret();
    builder.add_stmt(block, ret, mir_val);
    mir.add_function_definition(builder)
  }
}

#[derive(Debug)]
pub enum AstErrorVariant {
  Parser(ParserErrorVariant),
  MultipleValueDefinitions {
    name: String,
    original: Spanned<()>,
  },
}
pub type AstError = Spanned<AstErrorVariant>;
pub type AstResult<T> = Result<T, AstError>;

impl From<ParserError> for AstError {
  fn from(pe: ParserError) -> AstError {
    Spanned {
      thing: AstErrorVariant::Parser(pe.thing),
      start: pe.start,
      end: pe.end,
    }
  }
}

#[derive(Debug)]
pub struct Ast {
  funcs: HashMap<String, Function>,
}

impl Ast {
  pub fn new(file: &str) -> AstResult<Self> {
    let mut parse = Parser::new(file);
    let mut funcs = HashMap::<String, Function>::new();
    loop {
      match parse.next_item() {
        Ok((
          name,
          Spanned {
            thing: ItemVariant::Function(thing),
            start,
            end,
          },
        )) => {
          if let Some(orig) = funcs.get(&name) {
            return Err(Spanned {
              thing: AstErrorVariant::MultipleValueDefinitions {
                name: name.clone(),
                original: Spanned {
                  thing: (),
                  start: orig.start,
                  end: orig.end,
                },
              },
              start,
              end,
            });
          };
          funcs.insert(name, Spanned { thing, start, end });
        }
        Err(Spanned {
          thing: ParserErrorVariant::ExpectedEof,
          ..
        }) => break,
        Err(e) => return Err(e.into()),
      }
    }
    Ok(Ast { funcs })
  }
}

impl Ast {
  pub fn build_mir<'ctx>(&mut self, mir: &mut Mir<'ctx>) {
    Self::prelude_types(mir);
    let mut mir_funcs: HashMap<String, mir::FunctionDecl> =
      HashMap::new();
    for (name, _) in &self.funcs {
      let decl = mir.add_function_decl(Some(name.to_owned()));
      mir_funcs.insert(name.to_owned(), decl);
    }
    for (name, func) in &self.funcs {
      let decl = mir_funcs[name];
      func.build_mir(decl, &mir_funcs, mir);
    }
  }

  fn prelude_types(mir: &Mir) {
    mir.insert_type(
      Some(String::from("s32")),
      mir::TypeVariant::s32(),
    );
  }
}

impl Display for StringlyType {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      StringlyType::UserDefinedType(ref s) => write!(f, "{}", s),
      _ => panic!(),
    }
  }
}

impl Display for StatementVariant {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      StatementVariant::Expr(ref e) => {
        write!(f, "{}", **e)
      }
      StatementVariant::Local {
        ref name,
        ref ty,
        ref initializer,
      } => {
        write!(f, "{}: {} = {}", name, ty, **initializer)
      }
    }
  }
}

impl Display for BinOp {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      BinOp::Plus => {
        write!(f, "+")
      }
    }
  }
}

impl Display for ExpressionVariant {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    match *self {
      ExpressionVariant::IntLiteral(ref i) => {
        write!(f, "{}", i)
      }
      ExpressionVariant::Variable(ref s) => {
        write!(f, "{}", s)
      }
      ExpressionVariant::BinOp { ref lhs, ref rhs, ref op } => {
        write!(f, "{} {} {}", lhs.thing, op, rhs.thing)
      }
      ExpressionVariant::Call { ref callee } => {
        write!(f, "{}()", callee)
      }
      ExpressionVariant::Nullary => {
        write!(f, "()")
      }
    }
  }
}

impl Display for Ast {
  fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
    for (name, func) in &self.funcs {
      let ref func = func.thing;
      writeln!(f, "{} :: () -> {} {{", name, func.ret_ty)?;
      for stmt in &func.blk.statements {
        writeln!(f, "  {};", stmt.thing)?;
      }
      writeln!(f, "  {}", func.blk.expr.thing)?;
      writeln!(f, "}}")?;
    }
    Ok(())
  }
}
