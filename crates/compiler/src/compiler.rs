use crate::code::{Instructions, Opcode};
use crate::symbol_table::SymbolTable;
use lexer::token::Token;
use num_traits::FromPrimitive;
use object::object::{CompiledFunction, Object};
use parser::ast::Program;
use parser::ast::{BlockStatement, Conditional, Expression, InfixOperator, Primitive, Statement};

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct EmittedInstruction {
    opcode: Opcode,
    position: usize,
}

struct CompilerScope {
    instructions: Instructions,
    last_instruction: Option<EmittedInstruction>,
    previous_instruction: Option<EmittedInstruction>,
}

impl Default for CompilerScope {
    fn default() -> Self {
        Self::new()
    }
}

impl CompilerScope {
    pub fn new() -> Self {
        Self {
            instructions: Instructions::default(),
            last_instruction: None,
            previous_instruction: None,
        }
    }
}

pub struct Compiler {
    pub constants: Vec<Object>,

    pub symbol_table: SymbolTable,

    scopes: Vec<CompilerScope>,
    scope_index: usize,
}

impl Default for Compiler {
    fn default() -> Self {
        Self::new()
    }
}

impl Compiler {
    pub fn new() -> Self {
        let main_scope = CompilerScope::default();
        Compiler {
            constants: vec![],

            symbol_table: SymbolTable::default(),

            scopes: vec![main_scope],
            scope_index: 0,
        }
    }

    pub fn new_with_state(symbol_table: SymbolTable, constants: Vec<Object>) -> Self {
        let mut compiler = Compiler::new();
        compiler.symbol_table = symbol_table;
        compiler.constants = constants;
        compiler
    }

    pub fn compile(&mut self, program: Program) -> Result<(), String> {
        self.compile_statements(program.statements)
    }

    fn compile_block_statement(&mut self, block: BlockStatement) -> Result<(), String> {
        self.compile_statements(block.statements)
    }

    fn compile_statements(&mut self, statements: Vec<Statement>) -> Result<(), String> {
        for statement in statements {
            self.compile_statement(statement)?;
        }

        Ok(())
    }

    fn compile_statement(&mut self, statement: Statement) -> Result<(), String> {
        match statement {
            Statement::Expression(s) => {
                self.compile_expression(s)?;
                self.emit(Opcode::Pop, vec![]);
            }
            Statement::Let(s) => {
                self.compile_expression(s.value)?;

                let symbol = self.symbol_table.define(s.name.value);
                self.emit(Opcode::SetGlobal, vec![symbol.index as i32]);
            }
            Statement::Return(r) => {
                self.compile_expression(r.return_value)?;
                self.emit(Opcode::ReturnValue, vec![]);
            }
        }

        Ok(())
    }

    fn compile_expression(&mut self, expression: Expression) -> Result<(), String> {
        match expression {
            Expression::Infix(infix) => match infix.token {
                Token::LT | Token::LTE => self.compile_lt_and_lte(infix)?,
                _ => {
                    self.compile_expression(*infix.left)?;
                    self.compile_expression(*infix.right)?;
                    self.compile_infix_operator(&infix.token)?;
                }
            },
            Expression::Prefix(prefix) => {
                self.compile_expression(*prefix.right)?;
                self.compile_prefix_operator(&prefix.token)?;
            }
            Expression::Primitive(primitive) => self.compile_primitive(primitive)?,
            Expression::Conditional(conditional) => self.compile_conditional(conditional)?,
            Expression::Identifier(ident) => {
                let symbol = self.symbol_table.resolve(&ident.value);
                match symbol {
                    Some(symbol) => {
                        self.emit(Opcode::GetGlobal, vec![symbol.index as i32]);
                    }
                    None => {
                        return Err(format!("Undefined variable: {}", ident.value));
                    }
                }
            }
            Expression::ArrayLiteral(array) => {
                let len = i32::from_usize(array.elements.len()).ok_or("Invalid array length")?;
                for element in array.elements {
                    self.compile_expression(element)?;
                }
                self.emit(Opcode::Array, vec![len]);
            }

            Expression::HashMapLiteral(hasmap) => {
                let len = i32::from_usize(hasmap.pairs.len()).ok_or("Invalid hashmap length")?;
                for (key, value) in hasmap.pairs {
                    self.compile_expression(key)?;
                    self.compile_expression(value)?;
                }
                self.emit(Opcode::HashMap, vec![len * 2]);
            }
            Expression::IndexExpression(index) => {
                self.compile_expression(*index.left)?;
                self.compile_expression(*index.index)?;
                self.emit(Opcode::Index, vec![]);
            }
            Expression::FunctionLiteral(fun) => {
                self.enter_scope();
                self.compile_block_statement(fun.body)?;

                if self.last_instruction_is(Opcode::Pop) {
                    self.replace_last_pop_with_return();
                }

                let instructions = self.leave_scope().data;

                let compiled_function = Object::COMPILEDFUNCTION(CompiledFunction { instructions });
                let operands = i32::from_usize(self.add_constant(compiled_function))
                    .ok_or("Invalid integer type")?;
                self.emit(Opcode::Constant, vec![operands]);
            }
            Expression::FunctionCall(call) => {
                self.compile_expression(*call.function)?;

                self.emit(Opcode::Call, vec![]);
            }
        }

        Ok(())
    }

    fn compile_primitive(&mut self, primitive: Primitive) -> Result<(), String> {
        match primitive {
            Primitive::IntegerLiteral(i) => {
                let integer = Object::INTEGER(i);
                let pos = self.add_constant(integer);
                let pos = i32::from_usize(pos).ok_or("Invalid constant position")?;
                self.emit(Opcode::Constant, vec![pos]);
            }
            Primitive::BooleanLiteral(true) => {
                self.emit(Opcode::True, vec![]);
            }
            Primitive::BooleanLiteral(false) => {
                self.emit(Opcode::False, vec![]);
            }
            Primitive::StringLiteral(s) => {
                let string = Object::STRING(s);
                let pos = self.add_constant(string);
                let pos = i32::from_usize(pos).ok_or("Invalid constant position")?;
                self.emit(Opcode::Constant, vec![pos]);
            }
        }

        Ok(())
    }

    fn compile_infix_operator(&mut self, operator: &Token) -> Result<(), String> {
        match operator {
            Token::Plus => self.emit(Opcode::Add, vec![]),
            Token::Minus => self.emit(Opcode::Sub, vec![]),
            Token::Asterisk => self.emit(Opcode::Mul, vec![]),
            Token::Slash => self.emit(Opcode::Div, vec![]),
            Token::GT => self.emit(Opcode::GreaterThan, vec![]),
            Token::GTE => self.emit(Opcode::GreaterEqualThan, vec![]),
            Token::Equal => self.emit(Opcode::Equal, vec![]),
            Token::NotEqual => self.emit(Opcode::NotEqual, vec![]),
            Token::Or => self.emit(Opcode::Or, vec![]),
            Token::And => self.emit(Opcode::And, vec![]),
            _ => return Err(format!("Unknown operator: {operator}")),
        };
        Ok(())
    }

    fn compile_lt_and_lte(&mut self, infix: InfixOperator) -> Result<(), String> {
        self.compile_expression(*infix.right)?;
        self.compile_expression(*infix.left)?;
        match infix.token {
            Token::LT => self.emit(Opcode::GreaterThan, vec![]),
            Token::LTE => self.emit(Opcode::GreaterEqualThan, vec![]),
            tk => return Err(format!("Unknown operator: {tk}")),
        };
        Ok(())
    }

    fn compile_prefix_operator(&mut self, operator: &Token) -> Result<(), String> {
        match operator {
            Token::Bang => self.emit(Opcode::Bang, vec![]),
            Token::Minus => self.emit(Opcode::Minus, vec![]),
            _ => return Err(format!("Unknown operator: {operator}")),
        };
        Ok(())
    }

    fn compile_conditional(&mut self, conditional: Conditional) -> Result<(), String> {
        self.compile_expression(*conditional.condition)?;

        let jump_not_truthy_pos = self.emit(Opcode::JumpNotTruthy, vec![9999]); // We emit a dummy value for the jump offset
                                                                                // and we will fix it later
        self.compile_block_statement(conditional.consequence)?;
        if self.last_instruction_is(Opcode::Pop) {
            self.remove_last_instruction();
        }

        let jump_pos = self.emit(Opcode::Jump, vec![9999]); // We emit a dummy value for the jump offset
                                                            // and we will fix it later

        let after_consequence_pos = self.current_instructions().data.len();
        self.change_operand(jump_not_truthy_pos, after_consequence_pos as i32)?;

        if let Some(alternative) = conditional.alternative {
            self.compile_block_statement(alternative)?;
            if self.last_instruction_is(Opcode::Pop) {
                self.remove_last_instruction();
            }
        } else {
            self.emit(Opcode::Null, vec![]);
        }

        let after_alternative_pos = self.current_instructions().data.len();
        self.change_operand(jump_pos, after_alternative_pos as i32)?;

        Ok(())
    }

    fn last_instruction_is(&self, opcode: Opcode) -> bool {
        match self.scopes[self.scope_index].last_instruction {
            Some(ref last) => last.opcode == opcode,
            None => false,
        }
    }

    fn remove_last_instruction(&mut self) {
        if let Some(last) = self.scopes[self.scope_index].last_instruction.clone() {
            let previous = self.scopes[self.scope_index].previous_instruction.clone();

            let old = self.current_instructions().data.clone();
            let new = old[..last.position].to_vec();

            self.scopes[self.scope_index].instructions.data = new;
            self.scopes[self.scope_index].last_instruction = previous;
        }
    }

    fn add_constant(&mut self, obj: Object) -> usize {
        self.constants.push(obj);
        self.constants.len() - 1
    }

    fn emit(&mut self, opcode: Opcode, operands: Vec<i32>) -> usize {
        let instruction = opcode.make(operands);
        let pos = self.add_instruction(instruction);
        self.set_last_instruction(opcode, pos);
        pos
    }

    fn add_instruction(&mut self, instruction: Instructions) -> usize {
        let pos_new_instruction = self.current_instructions().data.len();
        self.scopes[self.scope_index]
            .instructions
            .append(instruction);
        pos_new_instruction
    }

    fn set_last_instruction(&mut self, opcode: Opcode, pos: usize) {
        let previous = self.scopes[self.scope_index].last_instruction.clone();
        let last = EmittedInstruction {
            opcode,
            position: pos,
        };
        self.scopes[self.scope_index].previous_instruction = previous;
        self.scopes[self.scope_index].last_instruction = Some(last);
    }

    fn change_operand(&mut self, pos: usize, operand: i32) -> Result<(), String> {
        let op = Opcode::from_u8(self.current_instructions().data[pos]).ok_or(format!(
            "Unknown opcode: {opcode}",
            opcode = self.current_instructions().data[pos]
        ))?;
        let new_instruction = op.make(vec![operand]);
        self.replace_instruction(pos, &new_instruction);
        Ok(())
    }

    fn replace_instruction(&mut self, pos: usize, new_instruction: &Instructions) {
        let ins = &mut self.scopes[self.scope_index].instructions;
        for (i, instruction) in new_instruction.data.iter().enumerate() {
            ins.data[pos + i] = *instruction;
        }
    }

    fn current_instructions(&self) -> Instructions {
        self.scopes[self.scope_index].instructions.clone()
    }

    fn enter_scope(&mut self) {
        let scope = CompilerScope::default();
        self.scopes.push(scope);
        self.scope_index += 1;
    }

    fn leave_scope(&mut self) -> Instructions {
        let instructions = self.current_instructions();

        self.scopes.pop();
        self.scope_index -= 1;

        instructions
    }

    fn replace_last_pop_with_return(&mut self) {
        let last_pos = self.scopes[self.scope_index]
            .last_instruction
            .as_ref()
            .unwrap()
            .position;
        self.replace_instruction(last_pos, &Opcode::ReturnValue.make(vec![]));
        self.scopes[self.scope_index]
            .last_instruction
            .as_mut()
            .unwrap()
            .opcode = Opcode::ReturnValue;
    }

    pub fn bytecode(&self) -> Bytecode {
        Bytecode::new(self.current_instructions(), self.constants.clone())
    }
}

pub struct Bytecode {
    pub instructions: Instructions,
    pub constants: Vec<Object>,
}

impl Bytecode {
    fn new(instructions: Instructions, constants: Vec<Object>) -> Self {
        Bytecode {
            instructions,
            constants,
        }
    }
}

#[cfg(test)]
pub mod tests {

    use super::*;

    #[test]
    fn test_compiler_scopes() {
        let mut compiler = Compiler::new();

        assert_eq!(compiler.scope_index, 0);
        compiler.emit(Opcode::Mul, vec![]);

        compiler.enter_scope();
        assert_eq!(compiler.scope_index, 1);

        compiler.emit(Opcode::Sub, vec![]);
        assert_eq!(
            compiler.scopes[compiler.scope_index]
                .instructions
                .data
                .len(),
            1
        );

        let last = compiler.scopes[compiler.scope_index]
            .last_instruction
            .clone()
            .unwrap();
        assert_eq!(last.opcode, Opcode::Sub);

        compiler.leave_scope();
        assert_eq!(compiler.scope_index, 0);

        compiler.emit(Opcode::Add, vec![]);
        assert_eq!(
            compiler.scopes[compiler.scope_index]
                .instructions
                .data
                .len(),
            2
        );

        let last = compiler.scopes[compiler.scope_index]
            .last_instruction
            .clone()
            .unwrap();
        assert_eq!(last.opcode, Opcode::Add);

        let previous = compiler.scopes[compiler.scope_index]
            .previous_instruction
            .clone()
            .unwrap();
        assert_eq!(previous.opcode, Opcode::Mul);
    }
}
