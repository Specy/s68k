use std::collections::{LinkedList, HashMap};

use serde::Serialize;
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

use crate::{
    instructions::{RegisterOperand, Size, Label},
    interpreter::Flags,
};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum MutationOperation {
    WriteRegister {
        register: RegisterOperand,
        old: u32,
        size: Size,
    },
    WriteMemory {
        address: usize,
        old: u32,
        size: Size,
    },
    WriteMemoryBytes {
        address: usize,
        old: Vec<u8>,
    },
    PushCall {
        to: usize,
        from: usize,
    },
    PopCall {
        to: usize,
        from: usize,
    }
}
#[derive(Serialize)]
pub struct ExecutionStep {
    mutations: Vec<MutationOperation>,
    pc: usize,
    line: usize,
    old_ccr: Flags,
    new_ccr: Flags,
}

impl ExecutionStep {
    pub fn new(pc: usize, ccr: Flags) -> Self {
        Self {
            mutations: vec![],
            pc,
            old_ccr: ccr,
            new_ccr: ccr,
            line: 0,
        }
    }
    pub fn add_mutation(&mut self, mutation: MutationOperation) {
        self.mutations.push(mutation);
    }
    pub fn set_pc(&mut self, pc: usize) {
        self.pc = pc;
    }
    pub fn set_ccr(&mut self, ccr: Flags) {
        self.old_ccr = ccr;
    }
    pub fn get_mutations(&self) -> &Vec<MutationOperation> {
        &self.mutations
    }
    pub fn get_pc(&self) -> usize {
        self.pc
    }
    pub fn get_ccr(&self) -> Flags {
        self.old_ccr
    }
}

pub struct CallStackFrame {
    address: usize,
    source_address: usize,
    registers: Vec<u32>,
}

impl CallStackFrame {
    pub fn new(address: usize, source_address: usize, registers: Vec<u32>) -> Self {
        Self {
            address,
            source_address,
            registers,
        }
    }
    pub fn get_address(&self) -> usize {
        self.address
    }
    pub fn get_source_address(&self) -> usize {
        self.source_address
    }
    pub fn get_registers(&self) -> Vec<u32> {
        self.registers.clone()
    }
}


#[wasm_bindgen]
pub struct Debugger {
    history: LinkedList<ExecutionStep>,
    history_size: usize,
    call_stack: Vec<CallStackFrame>,
    labels: HashMap<usize, Label>
}


impl Debugger {
    pub fn new(history_size: usize, labels: &HashMap<String, Label>) -> Self {
        let mut labels_map = HashMap::new();
        for label in labels.values() {
            labels_map.insert(label.address, label.clone());
        }
        //include at least one to prevent initialization errors when pushing history state
        let mut empty_history: LinkedList<ExecutionStep> = LinkedList::new();
        empty_history.push_front(ExecutionStep::new(0, Flags::empty()));
        Self {
            history: empty_history,
            history_size,
            call_stack: vec![],
            labels: labels_map
        }
    }
    pub fn add_step(&mut self, step: ExecutionStep) {
        self.history.push_back(step);
        if self.history.len() > self.history_size {
            self.history.pop_front();
        }
    }
    pub fn pop_step(&mut self) -> Option<ExecutionStep> {
        self.history.pop_back()
    }
    pub fn get_previous_mutations(&self) -> Option<&Vec<MutationOperation>> {
        match self.history.back() {
            Some(step) => Some(step.get_mutations()),
            None => None,
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.history.is_empty()
    }
    pub fn get_last_step(&self) -> Option<&ExecutionStep> {
        self.history.back()
    }
    pub fn set_new_ccr(&mut self, ccr: Flags) {
        self.history
            .back_mut()
            .expect("No history to set new ccr")
            .new_ccr = ccr;
    }
    pub fn set_line(&mut self, line: usize) {
        self.history
            .back_mut()
            .expect("No history to set new line")
            .line = line;
    }
    pub fn add_mutation(&mut self, operation: MutationOperation) {
        self.history
            .back_mut()
            .expect("No history to add mutation to")
            .add_mutation(operation);
    }
    pub fn get_history(&self) -> &LinkedList<ExecutionStep> {
        &self.history
    }
    pub fn get_last_steps(&self, count: usize) -> Vec<&ExecutionStep> {
        self.history
            .iter()
            .rev()
            .take(count)
            .collect::<Vec<&ExecutionStep>>()
    }
    pub fn get_labels(&self) -> &HashMap<usize, Label> {
        &self.labels
    }
    pub fn push_call(&mut self, address: usize, source_address: usize, registers: Vec<u32>) {
        self.call_stack.push(
            CallStackFrame::new(address, source_address, registers)
        );
    }
    pub fn pop_call(&mut self) -> Option<CallStackFrame> {
        self.call_stack.pop()
    }
    pub fn to_call_stack(&self) -> Vec<PrettyStackFrame> {
        self.call_stack.iter().map(|frame| {
            match self.labels.get(&frame.address) {
                Some(label) => PrettyStackFrame {
                    address: frame.address,
                    source_address: frame.source_address,
                    registers: frame.registers.clone(),
                    label_name: label.name.clone(),
                    label_address: label.address,
                    label_line: label.line
                },
                None => PrettyStackFrame {
                    address: frame.address,
                    source_address: frame.source_address,
                    registers: frame.registers.clone(),
                    label_name: "Unknown".to_string(),
                    label_address: frame.address,
                    label_line: 0,
                }
            }
        }).collect()
    }
}


#[derive(Debug, Clone, Serialize)]
pub struct PrettyStackFrame {
    pub address: usize,
    pub source_address: usize,
    pub registers: Vec<u32>,
    
    pub label_name: String,
    pub label_address: usize,
    pub label_line: usize,
}
#[wasm_bindgen(typescript_custom_section)]
const TS_STACK_FRAME: &'static str = r#"
export interface StackFrame {
    address: number,
    source_address: number,
    registers: number[],
    label_name: string,
    label_address: number,
    label_line: number
}
"#;